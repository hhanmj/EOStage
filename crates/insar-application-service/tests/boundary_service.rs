use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_application_service::{
    AdminBoundaryOptionsRequest, AdminBoundaryResolveRequest, AdminBoundarySearchRequest,
    ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
};
use insar_provider::boundary::{
    ChinaAdminOption, ChinaAdminOptions, ChinaBoundaryProvider, GlobalBoundaryProvider,
    ProviderBoundaryCandidate, ProviderBoundaryFeature,
};
use serde_json::json;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-boundary-service-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[derive(Clone)]
struct StaticGlobalProvider {
    searches: Arc<AtomicUsize>,
    resolves: Arc<AtomicUsize>,
    taiwan_searches: Arc<AtomicUsize>,
}

impl StaticGlobalProvider {
    fn feature(name: &str, object_id: &str, country_code: &str) -> ProviderBoundaryFeature {
        ProviderBoundaryFeature {
            provider_object_id: object_id.to_owned(),
            name: name.to_owned(),
            display_name: format!("{name}, provider fixture"),
            country_code: Some(country_code.to_owned()),
            admin_level: Some("state".to_owned()),
            geometry: json!({
                "type": "Polygon",
                "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]
            }),
            geometry_kind: "polygon".to_owned(),
            bbox: [100.0, 30.0, 101.0, 31.0],
            source: "OpenStreetMap Nominatim".to_owned(),
            attribution: Some("© OpenStreetMap contributors".to_owned()),
        }
    }

    fn candidate(name: &str, object_id: &str, country_code: &str) -> ProviderBoundaryCandidate {
        ProviderBoundaryCandidate {
            provider_object_id: object_id.to_owned(),
            name: name.to_owned(),
            display_name: format!("{name}, provider fixture"),
            country_code: Some(country_code.to_owned()),
            admin_level: Some("state".to_owned()),
            bbox: [100.0, 30.0, 101.0, 31.0],
            source: "OpenStreetMap Nominatim".to_owned(),
            attribution: Some("© OpenStreetMap contributors".to_owned()),
        }
    }
}

impl GlobalBoundaryProvider for StaticGlobalProvider {
    fn search(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        Ok(vec![Self::feature(query, "relation:1", "US")])
    }

    fn discover(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        if matches!(query, "tokyoo" | "日木") {
            return Ok(Vec::new());
        }
        Ok(vec![Self::candidate(query, "relation:1", "US")])
    }

    fn discover_china(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        Ok(vec![Self::candidate(query, "relation:2", "CN")])
    }

    fn resolve(
        &self,
        provider_object_id: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        self.resolves.fetch_add(1, Ordering::Relaxed);
        let (name, country_code) = match provider_object_id {
            "relation:449220" => ("臺灣", "TW"),
            "relation:1293250" => ("臺北市", "TW"),
            "relation:2497975" => ("南投縣", "TW"),
            _ => ("California", "US"),
        };
        Ok(Self::feature(name, provider_object_id, country_code))
    }

    fn search_taiwan(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.taiwan_searches.fetch_add(1, Ordering::Relaxed);
        Ok(vec![Self::feature(query, "relation:7108", "TW")])
    }
}

struct ForbiddenGlobalProvider;

impl GlobalBoundaryProvider for ForbiddenGlobalProvider {
    fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        panic!("domestic Taiwan boundary flow must never call global search")
    }

    fn discover(
        &self,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        panic!("domestic Taiwan boundary flow must never call global discover")
    }

    fn discover_china(
        &self,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        panic!("domestic Taiwan boundary flow must never call global China fallback")
    }

    fn resolve(
        &self,
        _provider_object_id: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        panic!("domestic Taiwan boundary flow must never call global resolve")
    }

    fn search_taiwan(
        &self,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        panic!("domestic Taiwan boundary flow must never call Nominatim search_taiwan")
    }
}

struct WrongChinaTaiwanIdentityProvider;

impl ChinaBoundaryProvider for WrongChinaTaiwanIdentityProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions::default())
    }

    fn resolve_exact_adcode(
        &self,
        _adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        Ok(ProviderBoundaryFeature {
            provider_object_id: "350000".to_owned(),
            name: "福建省".to_owned(),
            display_name: "福建省".to_owned(),
            country_code: Some("CN".to_owned()),
            admin_level: Some("province".to_owned()),
            geometry: json!({
                "type": "Polygon",
                "coordinates": [[[118.0, 24.0], [120.0, 24.0], [120.0, 27.0], [118.0, 24.0]]]
            }),
            geometry_kind: "polygon".to_owned(),
            bbox: [118.0, 24.0, 120.0, 27.0],
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
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.resolve_exact_adcode("350000").map(|f| vec![f])
    }
}

struct ForbiddenChinaProvider;

impl ChinaBoundaryProvider for ForbiddenChinaProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        panic!("Taiwan product options must not call the DataV provider")
    }

    fn resolve_exact_adcode(
        &self,
        _adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        panic!("Taiwan selector resolution must not call the DataV provider")
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        panic!("Taiwan selector resolution must not call the DataV provider")
    }
}

struct StaticChinaProvider;

impl ChinaBoundaryProvider for StaticChinaProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions {
            provinces: vec![
                ChinaAdminOption::identified("全国", "100000"),
                ChinaAdminOption::identified("湖北省", "420000"),
                ChinaAdminOption::identified("台湾省", "710000"),
            ],
            cities: vec![
                ChinaAdminOption::unidentified("全部"),
                ChinaAdminOption::identified("恩施土家族苗族自治州", "422800"),
            ],
            districts: vec![
                ChinaAdminOption::unidentified("全部"),
                ChinaAdminOption::identified("巴东县", "422823"),
            ],
        })
    }

    fn resolve_exact_adcode(
        &self,
        adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        let results = match adcode {
            "100000" => self.search("全国", "全部", "全部", "", 1)?,
            "710000" => self.search("台湾省", "全部", "全部", "", 1)?,
            "710800" => self.search("台湾省", "南投县", "全部", "", 1)?,
            "810000" => self.search("香港特别行政区", "全部", "全部", "", 1)?,
            "340100" => self.search("安徽省", "合肥市", "全部", "", 1)?,
            "310000" => self.search("上海市", "全部", "全部", "", 1)?,
            _ => self.search("湖北省", "恩施土家族苗族自治州", "巴东县", "", 1)?,
        };
        results.into_iter().next().ok_or_else(|| {
            insar_core::ContractError::new(
                "BOUNDARY_GEOMETRY_UNAVAILABLE",
                "fixture exact boundary missing",
                serde_json::Map::new(),
            )
            .expect("bounded fixture error")
        })
    }

    fn search(
        &self,
        province: &str,
        city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        if province == "全国" {
            return Ok(vec![ProviderBoundaryFeature {
                provider_object_id: "100000".to_owned(),
                name: "中国".to_owned(),
                display_name: "中国".to_owned(),
                country_code: Some("CN".to_owned()),
                admin_level: Some("country".to_owned()),
                geometry: json!({
                    "type": "Polygon",
                    "coordinates": [[[73.5, 18.0], [135.1, 18.0], [135.1, 53.6], [73.5, 18.0]]]
                }),
                geometry_kind: "polygon".to_owned(),
                bbox: [73.5, 18.0, 135.1, 53.6],
                source: "DataV areas_v3".to_owned(),
                attribution: None,
            }]);
        }
        if province == "香港特别行政区" {
            return Ok(vec![ProviderBoundaryFeature {
                provider_object_id: "810000".to_owned(),
                name: "香港特别行政区".to_owned(),
                display_name: "香港特别行政区".to_owned(),
                country_code: Some("CN".to_owned()),
                admin_level: Some("province".to_owned()),
                geometry: json!({
                    "type": "Polygon",
                    "coordinates": [[[113.8, 22.2], [114.4, 22.2], [114.4, 22.6], [113.8, 22.2]]]
                }),
                geometry_kind: "polygon".to_owned(),
                bbox: [113.8, 22.2, 114.4, 22.6],
                source: "DataV areas_v3".to_owned(),
                attribution: None,
            }]);
        }
        if province == "上海市" {
            return Ok(vec![ProviderBoundaryFeature {
                provider_object_id: "310000".to_owned(),
                name: "上海市".to_owned(),
                display_name: "上海市".to_owned(),
                country_code: Some("CN".to_owned()),
                admin_level: Some("province".to_owned()),
                geometry: json!({
                    "type": "Polygon",
                    "coordinates": [[[121.0, 30.8], [121.9, 30.8], [121.9, 31.8], [121.0, 30.8]]]
                }),
                geometry_kind: "polygon".to_owned(),
                bbox: [121.0, 30.8, 121.9, 31.8],
                source: "DataV areas_v3".to_owned(),
                attribution: None,
            }]);
        }
        if province == "安徽省" && city == "合肥市" {
            return Ok(vec![ProviderBoundaryFeature {
                provider_object_id: "340100".to_owned(),
                name: "合肥市".to_owned(),
                display_name: "安徽省 / 合肥市".to_owned(),
                country_code: Some("CN".to_owned()),
                admin_level: Some("city".to_owned()),
                geometry: json!({
                    "type": "Polygon",
                    "coordinates": [[[117.0, 31.5], [117.6, 31.5], [117.6, 32.1], [117.0, 31.5]]]
                }),
                geometry_kind: "polygon".to_owned(),
                bbox: [117.0, 31.5, 117.6, 32.1],
                source: "DataV areas_v3".to_owned(),
                attribution: None,
            }]);
        }
        if province == "台湾省" {
            let (provider_object_id, name, level) = if city == "全部" || city.is_empty() {
                ("710000", "台湾省", "province")
            } else {
                ("710800", city, "city")
            };
            return Ok(vec![ProviderBoundaryFeature {
                provider_object_id: provider_object_id.to_owned(),
                name: name.to_owned(),
                display_name: if level == "province" {
                    name.to_owned()
                } else {
                    format!("台湾省 / {name}")
                },
                country_code: Some("CN".to_owned()),
                admin_level: Some(level.to_owned()),
                geometry: json!({
                    "type": "Polygon",
                    "coordinates": [[[120.5, 23.5], [121.0, 23.5], [121.0, 24.0], [120.5, 23.5]]]
                }),
                geometry_kind: "polygon".to_owned(),
                bbox: [120.5, 23.5, 121.0, 24.0],
                source: "DataV areas_v3".to_owned(),
                attribution: None,
            }]);
        }
        Ok(vec![ProviderBoundaryFeature {
            provider_object_id: "422823".to_owned(),
            name: "巴东县".to_owned(),
            display_name: "湖北省 / 恩施土家族苗族自治州 / 巴东县".to_owned(),
            country_code: Some("CN".to_owned()),
            admin_level: Some("district".to_owned()),
            geometry: json!({
                "type": "MultiPolygon",
                "coordinates": [[[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 30.0]]]]
            }),
            geometry_kind: "multipolygon".to_owned(),
            bbox: [109.0, 30.0, 110.0, 31.0],
            source: "DataV areas_v3".to_owned(),
            attribution: None,
        }])
    }
}

#[derive(Clone)]
struct CountingChinaProvider {
    searches: Arc<AtomicUsize>,
    invalid_geometry: bool,
    provider_object_id: &'static str,
}

#[derive(Clone)]
struct FlakyExactChinaProvider {
    exact_resolves: Arc<AtomicUsize>,
    legacy_searches: Arc<AtomicUsize>,
}

impl ChinaBoundaryProvider for FlakyExactChinaProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions::default())
    }

    fn resolve_exact_adcode(
        &self,
        adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        let attempt = self.exact_resolves.fetch_add(1, Ordering::Relaxed);
        if attempt == 0 {
            return Err(insar_core::ContractError::new(
                "BOUNDARY_PROVIDER_HTTP_ERROR",
                "DataV transient fixture failure",
                serde_json::Map::new(),
            )
            .expect("bounded fixture error"));
        }
        Ok(ProviderBoundaryFeature {
            provider_object_id: adcode.to_owned(),
            name: "青海省".to_owned(),
            display_name: "青海省".to_owned(),
            country_code: Some("CN".to_owned()),
            admin_level: Some("province".to_owned()),
            geometry: json!({
                "type": "Polygon",
                "coordinates": [[[89.0, 31.0], [103.0, 31.0], [103.0, 39.0], [89.0, 39.0], [89.0, 31.0]]]
            }),
            geometry_kind: "polygon".to_owned(),
            bbox: [89.0, 31.0, 103.0, 39.0],
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
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.legacy_searches.fetch_add(1, Ordering::Relaxed);
        panic!("an identified DataV selector must resolve by exact adcode")
    }
}

const UNINDEXED_OPTION_NAME: &str = "DataV测试新区";
const UNINDEXED_OPTION_ADCODE: &str = "659012";

#[derive(Clone)]
struct UnindexedIdentityChinaProvider {
    searches: Arc<AtomicUsize>,
    returned_adcode: &'static str,
}

impl ChinaBoundaryProvider for UnindexedIdentityChinaProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions {
            provinces: vec![ChinaAdminOption::identified("DataV测试省", "650000")],
            cities: vec![ChinaAdminOption::identified("DataV测试市", "659000")],
            districts: vec![ChinaAdminOption::identified(
                UNINDEXED_OPTION_NAME,
                UNINDEXED_OPTION_ADCODE,
            )],
        })
    }

    fn resolve_exact_adcode(
        &self,
        _adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        self.search("", "", "", "", 1)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                insar_core::ContractError::new(
                    "BOUNDARY_GEOMETRY_UNAVAILABLE",
                    "fixture exact boundary missing",
                    serde_json::Map::new(),
                )
                .expect("bounded fixture error")
            })
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        Ok(vec![ProviderBoundaryFeature {
            provider_object_id: self.returned_adcode.to_owned(),
            name: UNINDEXED_OPTION_NAME.to_owned(),
            display_name: format!("DataV测试省 / DataV测试市 / {UNINDEXED_OPTION_NAME}"),
            country_code: Some("CN".to_owned()),
            admin_level: Some("district".to_owned()),
            geometry: json!({
                "type": "Polygon",
                "coordinates": [[[85.0, 42.0], [86.0, 42.0], [86.0, 43.0], [85.0, 43.0], [85.0, 42.0]]]
            }),
            geometry_kind: "polygon".to_owned(),
            bbox: [85.0, 42.0, 86.0, 43.0],
            source: "DataV areas_v3".to_owned(),
            attribution: None,
        }])
    }
}

impl ChinaBoundaryProvider for CountingChinaProvider {
    fn options(
        &self,
        _province: &str,
        _city: &str,
    ) -> insar_core::ProviderResult<ChinaAdminOptions> {
        Ok(ChinaAdminOptions::default())
    }

    fn resolve_exact_adcode(
        &self,
        _adcode: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        self.search("", "", "", "", 1)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                insar_core::ContractError::new(
                    "BOUNDARY_GEOMETRY_UNAVAILABLE",
                    "fixture exact boundary missing",
                    serde_json::Map::new(),
                )
                .expect("bounded fixture error")
            })
    }

    fn search(
        &self,
        _province: &str,
        _city: &str,
        _district: &str,
        _query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        let geometry = if self.invalid_geometry {
            json!({
                "type": "Polygon",
                "coordinates": [[[109.0, 30.0], [110.0, 31.0], [109.0, 31.0], [110.0, 30.0], [109.0, 30.0]]]
            })
        } else {
            json!({
                "type": "Polygon",
                "coordinates": [[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 31.0], [109.0, 30.0]]]
            })
        };
        Ok(vec![ProviderBoundaryFeature {
            provider_object_id: self.provider_object_id.to_owned(),
            name: "巴东县".to_owned(),
            display_name: "湖北省 / 恩施土家族苗族自治州 / 巴东县".to_owned(),
            country_code: Some("CN".to_owned()),
            admin_level: Some("district".to_owned()),
            geometry,
            geometry_kind: "polygon".to_owned(),
            bbox: [109.0, 30.0, 110.0, 31.0],
            source: "DataV areas_v3".to_owned(),
            attribution: None,
        }])
    }
}

#[derive(Clone)]
struct ChinaFallbackGlobalProvider {
    discoveries: Arc<AtomicUsize>,
    resolves: Arc<AtomicUsize>,
}

impl GlobalBoundaryProvider for ChinaFallbackGlobalProvider {
    fn search(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
        Ok(vec![StaticGlobalProvider::feature(
            query,
            "relation:41",
            "CN",
        )])
    }

    fn discover(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        Ok(vec![StaticGlobalProvider::candidate(
            query,
            "relation:41",
            "CN",
        )])
    }

    fn discover_china(
        &self,
        query: &str,
        _limit: usize,
    ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
        self.discoveries.fetch_add(1, Ordering::Relaxed);
        Ok(vec![
            StaticGlobalProvider::candidate(query, "relation:41", "CN"),
            StaticGlobalProvider::candidate(query, "relation:42", "CN"),
        ])
    }

    fn resolve(
        &self,
        provider_object_id: &str,
    ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
        self.resolves.fetch_add(1, Ordering::Relaxed);
        Ok(StaticGlobalProvider::feature(
            "巴东县",
            provider_object_id,
            "CN",
        ))
    }
}

#[test]
fn national_selector_routes_to_china_provider_as_an_exact_country_boundary() {
    let (service, global_searches, _, _) = service();
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "全国".to_owned(),
            city: "全部".to_owned(),
            district: "全部".to_owned(),
            provider_object_id: Some("100000".to_owned()),
            ..Default::default()
        })
        .expect("national selector search");

    assert_eq!(global_searches.load(Ordering::Relaxed), 0);
    assert_eq!(result.provider, "datav");
    assert_eq!(result.results[0].name, "中国");
    assert_eq!(result.results[0].admin_level.as_deref(), Some("country"));
    assert_eq!(result.results[0].provider_object_id, "100000");
}

fn service() -> (
    ApplicationService,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let searches = Arc::new(AtomicUsize::new(0));
    let resolves = Arc::new(AtomicUsize::new(0));
    let taiwan_searches = Arc::new(AtomicUsize::new(0));
    let global = StaticGlobalProvider {
        searches: Arc::clone(&searches),
        resolves: Arc::clone(&resolves),
        taiwan_searches: Arc::clone(&taiwan_searches),
    };
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(global),
        Arc::new(StaticChinaProvider),
    );
    (service, searches, resolves, taiwan_searches)
}

#[test]
fn global_search_discovers_candidates_then_resolves_the_selected_geometry() {
    let (service, searches, resolves, _) = service();
    let request = AdminBoundarySearchRequest {
        query: "California".to_owned(),
        ..Default::default()
    };

    let first = service
        .admin_boundary_search(&request)
        .expect("global search");
    let second = service
        .admin_boundary_search(&request)
        .expect("cached search");
    assert_eq!(searches.load(Ordering::Relaxed), 1);
    assert_eq!(first.results[0].provider, "nominatim");
    assert_eq!(first.results[0].boundary_id, "nominatim:relation:1");
    assert!(first.results[0].geometry.is_none());
    assert!(first.results[0].geometry_kind.is_none());
    assert_eq!(first.results[0].cache_status, "miss");
    assert_eq!(second.results[0].cache_status, "hit");
    assert_eq!(resolves.load(Ordering::Relaxed), 0);

    let resolved = service
        .admin_boundary_resolve(&AdminBoundaryResolveRequest {
            provider: "nominatim".to_owned(),
            provider_object_id: first.results[0].provider_object_id.clone(),
        })
        .expect("selected candidate resolves");
    assert_eq!(resolves.load(Ordering::Relaxed), 1);
    assert_eq!(resolved.provider_object_id, "relation:1");
    assert_eq!(resolved.geometry_kind.as_deref(), Some("polygon"));
    assert!(resolved.geometry.is_some());
    let cached_resolved = service
        .admin_boundary_resolve(&AdminBoundaryResolveRequest {
            provider: "nominatim".to_owned(),
            provider_object_id: first.results[0].provider_object_id.clone(),
        })
        .expect("selected candidate resolution cache");
    assert_eq!(resolves.load(Ordering::Relaxed), 1);
    assert_eq!(
        cached_resolved.provider_object_id,
        resolved.provider_object_id
    );
    assert_eq!(cached_resolved.cache_status, "hit");
}

#[test]
fn china_selector_uses_china_provider_and_returns_canonical_geometry() {
    let (service, _, _, _) = service();
    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
            district: "巴东县".to_owned(),
            provider_object_id: Some("422823".to_owned()),
            ..Default::default()
        })
        .expect("China selector search");
    assert_eq!(result.results[0].provider, "datav");
    assert_eq!(result.results[0].country_code.as_deref(), Some("CN"));
    assert_eq!(
        result.results[0].geometry_kind.as_deref(),
        Some("multipolygon")
    );
    assert_eq!(
        result.results[0].display_name,
        "湖北省 / 恩施土家族苗族自治州 / 巴东县"
    );
}

#[test]
fn taiwan_selector_resolves_the_verified_exact_relation_without_candidate_search() {
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(StaticChinaProvider),
    );
    let province_result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "全部".to_owned(),
            provider_object_id: Some("710000".to_owned()),
            ..Default::default()
        })
        .expect("Taiwan province DataV search");
    assert_eq!(province_result.provider, "datav");
    assert_eq!(province_result.warning, None);
    assert_eq!(province_result.results.len(), 1);
    assert_eq!(province_result.results[0].provider, "datav");
    assert_eq!(province_result.results[0].provider_object_id, "710000");
    assert_eq!(
        province_result.results[0].country_code.as_deref(),
        Some("CN")
    );
    assert_eq!(
        province_result.results[0].geometry_kind.as_deref(),
        Some("polygon")
    );
    assert!(province_result.results[0].geometry.is_some());

    // Without a personal Tianditu key, city selection falls back domestically to DataV 710000
    // with a warning and ZERO global/Nominatim requests.
    let city_result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "南投县".to_owned(),
            provider_object_id: Some("710800".to_owned()),
            ..Default::default()
        })
        .expect("Taiwan city domestic fallback search");
    assert_eq!(city_result.provider, "datav");
    assert!(city_result.warning.is_some());
    assert_eq!(city_result.results.len(), 1);
    assert_eq!(city_result.results[0].provider, "datav");
    assert_eq!(city_result.results[0].provider_object_id, "710000");
    assert_eq!(city_result.results[0].country_code.as_deref(), Some("CN"));
}

#[test]
fn taiwan_options_are_stable_product_owned_rows_with_verified_relation_identity() {
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(StaticGlobalProvider {
            searches: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::new(AtomicUsize::new(0)),
            taiwan_searches: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(ForbiddenChinaProvider),
    );
    let request = AdminBoundaryOptionsRequest {
        province: "台湾省".to_owned(),
        city: "全部".to_owned(),
    };
    let first = service
        .admin_options(&request)
        .expect("Taiwan product options");
    let second = service
        .admin_options(&request)
        .expect("stable Taiwan product options");
    let expected = vec![
        "全部",
        "台北市",
        "新北市",
        "桃园市",
        "台中市",
        "台南市",
        "高雄市",
        "基隆市",
        "新竹市",
        "嘉义市",
        "新竹县",
        "苗栗县",
        "彰化县",
        "南投县",
        "云林县",
        "嘉义县",
        "屏东县",
        "宜兰县",
        "花莲县",
        "台东县",
        "澎湖县",
        "金门县",
        "连江县",
    ];
    assert_eq!(first, second);
    assert_eq!(first.provider, "insar-studio-product-options");
    assert_eq!(first.warning, None);
    assert_eq!(
        first
            .cities
            .iter()
            .map(|option| option.name.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    let expected_identities = [
        ("全部", "datav", "710000"),
        ("台北市", "tianditu", "710100"),
        ("南投县", "tianditu", "710800"),
    ];
    for (name, expected_provider, provider_object_id) in expected_identities {
        let option = first
            .cities
            .iter()
            .find(|option| option.name == name)
            .expect("verified Taiwan option");
        assert_eq!(
            option.provider.as_deref(),
            Some(expected_provider),
            "{name}"
        );
        assert_eq!(
            option.provider_object_id.as_deref(),
            Some(provider_object_id),
            "{name}"
        );
    }
    assert!(first.cities.iter().all(|option| {
        matches!(option.provider.as_deref(), Some("datav") | Some("tianditu"))
            && option
                .provider_object_id
                .as_deref()
                .is_some_and(|identity| identity.len() == 6 && identity.starts_with("71"))
    }));
    assert_eq!(first.districts.len(), 1);
    assert_eq!(first.districts[0].name, "全部");
}

#[test]
fn taiwan_resolved_polygon_is_persisted_and_reused_by_exact_provider_identity_after_restart() {
    let database = TempDatabase::new("taiwan-datav-persistent-cache");
    let first = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::store(database.path()),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(StaticChinaProvider),
    );
    let selected = first
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "全部".to_owned(),
            provider_object_id: Some("710000".to_owned()),
            ..Default::default()
        })
        .expect("persist Taiwan selector polygon")
        .results
        .into_iter()
        .next()
        .expect("one Taiwan polygon");
    assert_eq!(selected.provider, "datav");
    assert_eq!(selected.provider_object_id, "710000");
    drop(first);

    let second = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::existing_store(database.path()),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(ForbiddenChinaProvider),
    );
    let cached = second
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "全部".to_owned(),
            provider_object_id: Some(selected.provider_object_id),
            ..Default::default()
        })
        .expect("Taiwan selector cache hit after restart")
        .results
        .into_iter()
        .next()
        .expect("cached Taiwan boundary");
    assert_eq!(cached.cache_status, "hit");
    assert_eq!(cached.provider, "datav");
    assert_eq!(cached.country_code.as_deref(), Some("CN"));
}

#[test]
fn taiwan_selector_rejects_missing_or_mismatched_product_identity() {
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(StaticChinaProvider),
    );
    for provider_object_id in [
        None,
        Some("710100".to_owned()),
        Some("relation:1293250".to_owned()),
    ] {
        let error = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                province: "台湾省".to_owned(),
                city: "南投县".to_owned(),
                provider_object_id,
                ..Default::default()
            })
            .expect_err("Taiwan selector must fail closed without the verified identity");
        match error {
            ApplicationServiceError::Provider(provider) => {
                assert_eq!(provider.code, "BOUNDARY_PROVIDER_IDENTITY_INVALID");
                assert_eq!(
                    provider.debug_context["reason"],
                    "taiwan_provider_object_id"
                );
            }
            other => panic!("unexpected Taiwan identity error: {other:?}"),
        }
    }
}

#[test]
fn taiwan_selector_rejects_a_provider_response_with_another_relation_identity() {
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(WrongChinaTaiwanIdentityProvider),
    );
    let error = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "全部".to_owned(),
            provider_object_id: Some("710000".to_owned()),
            ..Default::default()
        })
        .expect_err("a different returned DataV adcode must fail exact Taiwan identity");
    match error {
        ApplicationServiceError::Provider(provider) => {
            assert_eq!(provider.code, "BOUNDARY_PROVIDER_IDENTITY_MISMATCH");
            assert_eq!(
                provider.debug_context["reason"],
                "taiwan_exact_administrative_identity"
            );
        }
        other => panic!("unexpected Taiwan response error: {other:?}"),
    }
}

#[test]
fn china_options_do_not_require_region_context() {
    let (service, _, _, _) = service();
    let options = service
        .admin_options(&AdminBoundaryOptionsRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
        })
        .expect("China options");
    assert!(options.cities.iter().any(|option| {
        option.name == "恩施土家族苗族自治州"
            && option.provider_object_id.as_deref() == Some("422800")
    }));
    assert!(options.districts.iter().any(|option| {
        option.name == "巴东县" && option.provider_object_id.as_deref() == Some("422823")
    }));
}

#[test]
fn datav_option_absent_from_legacy_index_preserves_its_adcode_end_to_end() {
    assert!(
        !include_str!("../../../ui/src/data/adminBoundaries.ts").contains(UNINDEXED_OPTION_NAME)
    );
    let searches = Arc::new(AtomicUsize::new(0));
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(UnindexedIdentityChinaProvider {
            searches: Arc::clone(&searches),
            returned_adcode: UNINDEXED_OPTION_ADCODE,
        }),
    );
    let options = service
        .admin_options(&AdminBoundaryOptionsRequest {
            province: "DataV测试省".to_owned(),
            city: "DataV测试市".to_owned(),
        })
        .expect("DataV options");
    let selected = options
        .districts
        .iter()
        .find(|option| option.name == UNINDEXED_OPTION_NAME)
        .expect("unindexed DataV option");
    assert_eq!(
        selected.provider_object_id.as_deref(),
        Some(UNINDEXED_OPTION_ADCODE)
    );

    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "DataV测试省".to_owned(),
            city: "DataV测试市".to_owned(),
            district: selected.name.clone(),
            provider_object_id: selected.provider_object_id.clone(),
            ..Default::default()
        })
        .expect("exact DataV selector identity");
    assert_eq!(searches.load(Ordering::Relaxed), 1);
    assert_eq!(result.provider, "datav");
    assert_eq!(
        result.results[0].provider_object_id,
        UNINDEXED_OPTION_ADCODE
    );
}

#[test]
fn invalid_datav_geometry_falls_back_to_explicit_china_nominatim_candidates() {
    let datav_searches = Arc::new(AtomicUsize::new(0));
    let fallback_discoveries = Arc::new(AtomicUsize::new(0));
    let fallback_resolves = Arc::new(AtomicUsize::new(0));
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::clone(&fallback_discoveries),
            resolves: Arc::clone(&fallback_resolves),
        }),
        Arc::new(CountingChinaProvider {
            searches: Arc::clone(&datav_searches),
            invalid_geometry: true,
            provider_object_id: "422823",
        }),
    );

    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
            district: "巴东县".to_owned(),
            provider_object_id: Some("422823".to_owned()),
            ..Default::default()
        })
        .expect("fallback candidates");

    assert_eq!(datav_searches.load(Ordering::Relaxed), 1);
    assert_eq!(fallback_discoveries.load(Ordering::Relaxed), 1);
    assert_eq!(result.provider, "nominatim");
    assert_eq!(result.results.len(), 2);
    assert!(result.results.iter().all(|candidate| {
        candidate.provider == "nominatim"
            && candidate.provider_object_id.starts_with("relation:")
            && candidate.geometry.is_none()
    }));
    assert!(result.warning.as_deref().is_some_and(|warning| {
        warning.contains("DataV") && warning.contains("OSM") && warning.contains("选择")
    }));

    let resolved = service
        .admin_boundary_resolve(&AdminBoundaryResolveRequest {
            provider: result.results[1].provider.clone(),
            provider_object_id: result.results[1].provider_object_id.clone(),
        })
        .expect("explicitly selected OSM fallback polygon");
    assert_eq!(fallback_resolves.load(Ordering::Relaxed), 1);
    assert_eq!(resolved.provider, "nominatim");
    assert_eq!(resolved.provider_object_id, "relation:42");
    assert_eq!(resolved.geometry_kind.as_deref(), Some("polygon"));
    assert!(resolved.geometry.is_some());
}

#[test]
fn mismatched_datav_selector_identity_falls_back_to_china_nominatim_candidates() {
    let datav_searches = Arc::new(AtomicUsize::new(0));
    let fallback_discoveries = Arc::new(AtomicUsize::new(0));
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::clone(&fallback_discoveries),
            resolves: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(CountingChinaProvider {
            searches: Arc::clone(&datav_searches),
            invalid_geometry: false,
            provider_object_id: "422822",
        }),
    );

    let result = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "湖北省".to_owned(),
            city: "恩施土家族苗族自治州".to_owned(),
            district: "巴东县".to_owned(),
            provider_object_id: Some("422823".to_owned()),
            ..Default::default()
        })
        .expect("identity mismatch fallback candidates");

    assert_eq!(datav_searches.load(Ordering::Relaxed), 1);
    assert_eq!(fallback_discoveries.load(Ordering::Relaxed), 1);
    assert_eq!(result.provider, "nominatim");
    assert!(result
        .results
        .iter()
        .all(|candidate| candidate.provider_object_id.starts_with("relation:")));
}

#[test]
fn datav_fallback_candidates_do_not_cache_over_the_exact_selector_identity() {
    let exact_resolves = Arc::new(AtomicUsize::new(0));
    let legacy_searches = Arc::new(AtomicUsize::new(0));
    let fallback_discoveries = Arc::new(AtomicUsize::new(0));
    let service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::clone(&fallback_discoveries),
            resolves: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(FlakyExactChinaProvider {
            exact_resolves: Arc::clone(&exact_resolves),
            legacy_searches: Arc::clone(&legacy_searches),
        }),
    );
    let request = AdminBoundarySearchRequest {
        province: "青海省".to_owned(),
        city: "全部".to_owned(),
        district: "全部".to_owned(),
        provider_object_id: Some("630000".to_owned()),
        ..Default::default()
    };

    let fallback = service
        .admin_boundary_search(&request)
        .expect("first transient failure exposes bounded fallback candidates");
    assert_eq!(fallback.provider, "nominatim");

    let recovered = service
        .admin_boundary_search(&request)
        .expect("second request retries exact DataV identity");
    assert_eq!(recovered.provider, "datav");
    assert_eq!(recovered.results[0].provider_object_id, "630000");
    assert_eq!(exact_resolves.load(Ordering::Relaxed), 2);
    assert_eq!(legacy_searches.load(Ordering::Relaxed), 0);
    assert_eq!(fallback_discoveries.load(Ordering::Relaxed), 1);
}

#[test]
fn unindexed_datav_option_hits_persistent_cache_after_restart_without_provider_request() {
    let database = TempDatabase::new("datav-persistent-cache");
    let first_searches = Arc::new(AtomicUsize::new(0));
    let first = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::store(database.path()),
        Arc::new(StaticGlobalProvider {
            searches: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::new(AtomicUsize::new(0)),
            taiwan_searches: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(UnindexedIdentityChinaProvider {
            searches: Arc::clone(&first_searches),
            returned_adcode: UNINDEXED_OPTION_ADCODE,
        }),
    );
    let request = AdminBoundarySearchRequest {
        province: "DataV测试省".to_owned(),
        city: "DataV测试市".to_owned(),
        district: UNINDEXED_OPTION_NAME.to_owned(),
        provider_object_id: Some(UNINDEXED_OPTION_ADCODE.to_owned()),
        ..Default::default()
    };
    let first_result = first
        .admin_boundary_search(&request)
        .expect("first DataV resolution");
    assert_eq!(first_result.results[0].cache_status, "miss");
    assert_eq!(first_searches.load(Ordering::Relaxed), 1);
    drop(first);

    let second_searches = Arc::new(AtomicUsize::new(0));
    let second = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::existing_store(database.path()),
        Arc::new(StaticGlobalProvider {
            searches: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::new(AtomicUsize::new(0)),
            taiwan_searches: Arc::new(AtomicUsize::new(0)),
        }),
        Arc::new(UnindexedIdentityChinaProvider {
            searches: Arc::clone(&second_searches),
            returned_adcode: "659011",
        }),
    );
    let cached = second
        .admin_boundary_search(&request)
        .expect("persistent DataV cache hit");
    assert_eq!(cached.results[0].provider, "datav");
    assert_eq!(
        cached.results[0].provider_object_id,
        UNINDEXED_OPTION_ADCODE
    );
    assert_eq!(cached.results[0].cache_status, "hit");
    assert_eq!(second_searches.load(Ordering::Relaxed), 0);
}

#[test]
fn nominatim_resolved_boundary_survives_restart_and_skips_resolve_network() {
    let database = TempDatabase::new("nominatim-persistent-cache");
    let first_resolves = Arc::new(AtomicUsize::new(0));
    let first = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::store(database.path()),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::clone(&first_resolves),
        }),
        Arc::new(StaticChinaProvider),
    );
    let request = AdminBoundaryResolveRequest {
        provider: "nominatim".to_owned(),
        provider_object_id: "relation:41".to_owned(),
    };
    let first_result = first
        .admin_boundary_resolve(&request)
        .expect("first OSM resolution");
    assert_eq!(first_result.cache_status, "miss");
    assert_eq!(first_resolves.load(Ordering::Relaxed), 1);
    drop(first);

    let second_resolves = Arc::new(AtomicUsize::new(0));
    let second = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::existing_store(database.path()),
        Arc::new(ChinaFallbackGlobalProvider {
            discoveries: Arc::new(AtomicUsize::new(0)),
            resolves: Arc::clone(&second_resolves),
        }),
        Arc::new(StaticChinaProvider),
    );
    let cached = second
        .admin_boundary_resolve(&request)
        .expect("persistent OSM cache hit");
    assert_eq!(cached.provider, "nominatim");
    assert_eq!(cached.provider_object_id, "relation:41");
    assert_eq!(cached.cache_status, "hit");
    assert_eq!(second_resolves.load(Ordering::Relaxed), 0);
}

#[test]
fn datav_resolve_refusal_keeps_its_code_and_names_the_china_selector() {
    let (service, _, resolves, _) = service();
    let error = service
        .admin_boundary_resolve(&AdminBoundaryResolveRequest {
            provider: "datav".to_owned(),
            provider_object_id: "810000".to_owned(),
        })
        .expect_err("DataV candidates are not resolved here");
    // L4 r4: this reached the desktop as V3_PROVIDER_OPERATION_FAILED.
    assert_eq!(error.code(), "BOUNDARY_PROVIDER_IDENTITY_INVALID");
    match &error {
        ApplicationServiceError::Provider(source) => {
            assert_eq!(source.debug_context["fallback"], "china_selector");
            assert!(
                source.user_message.contains("中国行政区选择器"),
                "{}",
                source.user_message
            );
        }
        other => panic!("expected a provider error, got {other:?}"),
    }
    assert_eq!(resolves.load(Ordering::Relaxed), 0);

    for refusal in ["CHN", "HKG"] {
        let error = ApplicationServiceError::Provider(
            insar_provider::geoboundaries::geoboundaries_route_refusal(refusal)
                .expect("China scope is refused"),
        );
        assert_eq!(error.code(), "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE");
    }
    let disputed = ApplicationServiceError::Provider(
        insar_provider::geoboundaries::disputed_route_refusal("disputed_guard_region"),
    );
    assert_eq!(disputed.code(), "BOUNDARY_ROUTE_REFUSED_DISPUTED");
}

#[test]
fn china_scope_text_queries_follow_the_china_selector_route() {
    let (service, searches, resolves, _) = service();
    // L4 r4: 香港 went to Nominatim and returned two objects of the same name.
    for query in [
        "香港",
        "香港特别行政区",
        "Hong Kong",
        "香港特区",
        "香港特別行政區",
    ] {
        let result = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                query: query.to_owned(),
                ..Default::default()
            })
            .expect("China-scope text search");
        assert_eq!(result.provider, "datav", "{query}");
        assert_eq!(result.query, query);
        assert_eq!(result.results.len(), 1, "{query}");
        assert_eq!(result.results[0].provider_object_id, "810000", "{query}");
    }
    assert_eq!(searches.load(Ordering::Relaxed), 0);

    // Taiwan text uses domestic DataV 710000 and never calls Nominatim.
    let taiwan = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "台湾".to_owned(),
            ..Default::default()
        })
        .expect("Taiwan text search");
    assert_eq!(taiwan.provider, "datav");
    assert_eq!(taiwan.results.len(), 1);
    assert_eq!(taiwan.results[0].provider_object_id, "710000");
    assert_eq!(resolves.load(Ordering::Relaxed), 0);
    assert_eq!(searches.load(Ordering::Relaxed), 0);

    // Partial names and foreign places stay on free-text search.
    for query in ["香港仔", "Kathmandu"] {
        let result = service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                query: query.to_owned(),
                ..Default::default()
            })
            .expect("free-text search");
        assert_eq!(result.provider, "nominatim", "{query}");
    }
    assert_eq!(searches.load(Ordering::Relaxed), 2);
}

#[test]
fn free_text_admin_aliases_and_typos_preserve_original_query_and_keep_taiwan_domestic() {
    let (service, searches, resolves, _) = service();

    // Colloquial country alias: "小日本" -> global query "日本" while keeping response.query == "小日本"
    let japan_alias = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "小日本".to_owned(),
            ..Default::default()
        })
        .expect("colloquial country alias search");
    assert_eq!(japan_alias.query, "小日本");
    assert_eq!(japan_alias.provider, "nominatim");
    assert_eq!(japan_alias.results[0].name, "日本");
    assert_eq!(searches.load(Ordering::Relaxed), 1);
    assert_eq!(resolves.load(Ordering::Relaxed), 0);

    // Typo 1-edit-distance fallback ("日木" / "tokyoo" return empty -> retry canonical target)
    let japan_typo = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "日木".to_owned(),
            ..Default::default()
        })
        .expect("CJK 1-char substitution typo fallback");
    assert_eq!(japan_typo.query, "日木");
    assert_eq!(japan_typo.results[0].name, "日本");

    let tokyo_typo = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "tokyoo".to_owned(),
            ..Default::default()
        })
        .expect("Latin 1-edit typo fallback");
    assert_eq!(tokyo_typo.query, "tokyoo");
    assert_eq!(tokyo_typo.results[0].name, "Tokyo");

    // Colloquial China province & city nicknames use domestic DataV without touching global search.
    let searches_before_china = searches.load(Ordering::Relaxed);
    let modu = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "魔都".to_owned(),
            ..Default::default()
        })
        .expect("Shanghai nickname");
    assert_eq!(modu.query, "魔都");
    assert_eq!(modu.provider, "datav");
    assert_eq!(modu.results[0].provider_object_id, "310000");

    let badu = service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            query: "霸都".to_owned(),
            ..Default::default()
        })
        .expect("Hefei nickname");
    assert_eq!(badu.query, "霸都");
    assert_eq!(badu.provider, "datav");
    assert_eq!(badu.results[0].provider_object_id, "340100");
    assert_eq!(searches.load(Ordering::Relaxed), searches_before_china);

    // Taiwan aliases, English names, historical counties, and Taiwan typos NEVER call global Nominatim.
    let domestic_service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(ForbiddenGlobalProvider),
        Arc::new(StaticChinaProvider),
    );
    for query in [
        "台湾",
        "台灣",
        "宝岛",
        "台北",
        "臺北市",
        "taiwan",
        "taipei",
        "kaohsiung",
        "台北县",
        "高雄县",
        "台北士",
        "台弯省",
        "taipai",
    ] {
        let tw = domestic_service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                query: query.to_owned(),
                ..Default::default()
            })
            .expect("Taiwan free-text must stay domestic");
        assert_eq!(tw.query, query);
        assert_eq!(tw.provider, "datav", "{query}");
        assert_eq!(tw.results[0].provider_object_id, "710000", "{query}");
    }
    // Pre-selected Taiwan province + city still resolves typos ("taipai") and province resets ("taiwan", "台弯省") domestically.
    let preselected_reset = domestic_service
        .admin_boundary_search(&AdminBoundarySearchRequest {
            province: "台湾省".to_owned(),
            city: "高雄市".to_owned(),
            query: "taiwan".to_owned(),
            ..Default::default()
        })
        .expect("preselected Taiwan province reset stays domestic");
    assert_eq!(preselected_reset.provider, "datav");
    assert_eq!(preselected_reset.results[0].provider_object_id, "710000");

    // Real 2-char/3-char Chinese place names MUST NOT be silently hijacked by CJK typo fallback!
    struct EmptyGlobalProvider;
    impl GlobalBoundaryProvider for EmptyGlobalProvider {
        fn search(
            &self,
            _query: &str,
            _limit: usize,
        ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
            Ok(Vec::new())
        }
        fn discover(
            &self,
            _query: &str,
            _limit: usize,
        ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
            Ok(Vec::new())
        }
        fn discover_china(
            &self,
            _query: &str,
            _limit: usize,
        ) -> insar_core::ProviderResult<Vec<ProviderBoundaryCandidate>> {
            Ok(Vec::new())
        }
        fn resolve(
            &self,
            _provider_object_id: &str,
        ) -> insar_core::ProviderResult<ProviderBoundaryFeature> {
            unreachable!("empty discover test never resolves a global candidate")
        }
        fn search_taiwan(
            &self,
            _query: &str,
            _limit: usize,
        ) -> insar_core::ProviderResult<Vec<ProviderBoundaryFeature>> {
            Ok(Vec::new())
        }
    }
    let empty_global_service = ApplicationService::with_boundary_providers(
        ApplicationServiceConfig::liveness_only(),
        Arc::new(EmptyGlobalProvider),
        Arc::new(StaticChinaProvider),
    );
    for real_city in [
        "日照",
        "德阳",
        "德州",
        "韩城",
        "英德",
        "南宁市",
        "长治市",
        "淮安市",
        "广安市",
        "台州市",
        "香港仔",
    ] {
        let res = empty_global_service
            .admin_boundary_search(&AdminBoundarySearchRequest {
                query: real_city.to_owned(),
                ..Default::default()
            })
            .expect("real Chinese city query should complete without error");
        assert_eq!(res.query, real_city);
        assert!(
            res.results.is_empty(),
            "real city {real_city} must NEVER falsely resolve to another region: {:?}",
            res.results
        );
    }
}
