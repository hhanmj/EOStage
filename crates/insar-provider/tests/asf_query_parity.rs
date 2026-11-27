use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use insar_core::{
    AsfRemoteSearchRequest, AsfSearchQuery, ContractError, ProviderResult, SarSearchQuery,
};
use insar_provider::asf::cmr::cmr_page_params;
use insar_provider::asf::query::{
    apply_remote_count, bisect_window, next_refill_limit, plan_asf_query, plan_sar_query,
    split_query_windows, AoiPlanMode, QueryWindow,
};
use insar_provider::asf::search::{AsfProvider, MetadataTransport};
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    split_windows: WindowCase,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    query: AsfSearchQuery,
    expected_requested_limit: u64,
    expected_query_limit: u64,
    expected_count: bool,
    expected_aoi_mode: String,
    expected_params: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct WindowCase {
    start: String,
    end: String,
    now: String,
    expected: Vec<(String, String)>,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/v3_asf/asf_query_plans.json");
    serde_json::from_str(&fs::read_to_string(path).expect("fixture exists"))
        .expect("fixture is valid")
}

#[test]
fn request_plans_match_python_v219_fixture() {
    for case in fixture().cases {
        let plan =
            plan_asf_query(&case.query).unwrap_or_else(|error| panic!("{}: {error}", case.name));
        assert_eq!(
            plan.requested_limit, case.expected_requested_limit,
            "{}",
            case.name
        );
        assert_eq!(plan.query_limit, case.expected_query_limit, "{}", case.name);
        assert_eq!(plan.should_count, case.expected_count, "{}", case.name);
        assert_eq!(
            plan.aoi_mode.as_str(),
            case.expected_aoi_mode,
            "{}",
            case.name
        );
        assert_eq!(plan.params, case.expected_params, "{}", case.name);
    }
}

#[test]
fn date_windows_match_python_v219_fixture() {
    let case = fixture().split_windows;
    let windows = split_query_windows(Some(&case.start), Some(&case.end), &case.now)
        .expect("valid fixture window");
    let actual = windows
        .into_iter()
        .map(|window| (window.start, window.end))
        .collect::<Vec<_>>();
    assert_eq!(actual, case.expected);
}

fn query_with(overrides: serde_json::Value) -> AsfSearchQuery {
    let mut value = json!({
        "version": 1,
        "dataset": "SENTINEL-1",
        "bbox": null,
        "aoi": "",
        "aoi_geojson": null,
        "start": "",
        "end": "",
        "product_type": "SLC",
        "beam_mode": "IW",
        "polarization": "",
        "orbit_direction": "",
        "relative_orbit": null,
        "frame": null,
        "max_results": null
    });
    value
        .as_object_mut()
        .expect("query object")
        .extend(overrides.as_object().expect("override object").clone());
    serde_json::from_value(value).expect("valid test query")
}

fn new_zealand_geometry() -> Map<String, Value> {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[166,-47],[179,-47],[179,-34],[166,-34],[166,-47]]],
            [[[-179,-45],[-175,-45],[-175,-35],[-179,-35],[-179,-45]]]
        ]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn new_zealand_query() -> SarSearchQuery {
    serde_json::from_value(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "beam_modes": ["IW"],
        "polarizations": ["VV+VH"],
        "aoi_geojson": new_zealand_geometry(),
        "start": "2024-01-01",
        "end": "2024-01-31",
        "max_results": 20
    }))
    .unwrap()
}

fn new_zealand_legacy_query() -> AsfSearchQuery {
    query_with(json!({
        "aoi": "New Zealand",
        "aoi_geojson": new_zealand_geometry(),
        "start": "2024-01-01",
        "end": "2024-01-31",
        "product_type": "SLC",
        "beam_mode": "IW",
        "polarization": "DV",
        "orbit_direction": "",
        "relative_orbit": "",
        "frame": "",
        "max_results": 20
    }))
}

fn cmr_entry(scene_id: &str, west: f64, east: f64) -> Value {
    json!({
        "producer_granule_id": scene_id,
        "links": [],
        "polygons": [[format!("-45 {west} -45 {east} -35 {east} -35 {west} -45 {west}")]]
    })
}

fn scripted_asf_error(message: &str) -> ContractError {
    ContractError::new("ASF002", message, Map::new()).unwrap()
}

fn generated_scene_id(index: usize) -> String {
    format!(
        "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_{:06}_064ABC_{:04X}",
        52_000 + index,
        index
    )
}

const MIXED_CASE_UPPER: &str =
    "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_A001";
const MIXED_CASE_LOWER_ALIAS: &str =
    "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064abc_a001.SAFE";
const MIXED_CASE_LOWER_DISPLAY: &str =
    "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064abc_a001";

#[derive(Default)]
struct CrossingCmrTransport {
    asf_responses: Mutex<VecDeque<ProviderResult<Value>>>,
    cmr_responses: Mutex<VecDeque<Value>>,
    cmr_calls: Mutex<Vec<Vec<(String, String)>>>,
}

impl MetadataTransport for CrossingCmrTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.asf_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(scripted_asf_error("scripted ASF failure")))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(None)
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_calls.lock().unwrap().push(params.to_vec());
        Ok(self
            .cmr_responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted CMR response"))
    }
}

#[derive(Default)]
struct FixedPageCmrTransport {
    cmr_calls: Mutex<Vec<Vec<(String, String)>>>,
}

impl MetadataTransport for FixedPageCmrTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        Err(scripted_asf_error("scripted ASF failure"))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(400))
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_calls.lock().unwrap().push(params.to_vec());
        let value = |name: &str| {
            params
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };
        let page_size = value("page_size").parse::<usize>().unwrap();
        let page_number = value("page_num").parse::<u64>().unwrap();
        let bbox = value("bounding_box");
        if page_size != 200 {
            return Err(scripted_asf_error(
                "CMR page_size changed while page_num advanced",
            ));
        }

        let entries = match (bbox, page_number) {
            ("166,-47,180,-34", 1) => (0..200)
                .map(|index| cmr_entry(&generated_scene_id(index), 166.0, 179.0))
                .collect(),
            ("-180,-47,-175,-34", 1) => std::iter::once(0)
                .chain(200..399)
                .map(|index| cmr_entry(&generated_scene_id(index), -179.0, -175.0))
                .collect(),
            ("166,-47,180,-34", 2) => {
                vec![cmr_entry(&generated_scene_id(399), 166.0, 179.0)]
            }
            ("-180,-47,-175,-34", 2) => Vec::new(),
            _ => panic!("unexpected CMR request: bbox={bbox}, page={page_number}"),
        };
        Ok(json!({"feed": {"entry": entries}}))
    }
}

#[derive(Default)]
struct MixedCasePagedCmrTransport {
    cmr_calls: Mutex<Vec<Vec<(String, String)>>>,
    enrichment_calls: Mutex<Vec<String>>,
}

impl MetadataTransport for MixedCasePagedCmrTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        let Some(granule_list) = params.get("granule_list") else {
            return Err(scripted_asf_error("scripted primary failure"));
        };
        self.enrichment_calls
            .lock()
            .unwrap()
            .push(granule_list.clone());
        let upper_slc = format!("{MIXED_CASE_UPPER}-SLC");
        let includes_alias = granule_list
            .split(',')
            .any(|identifier| identifier == MIXED_CASE_UPPER || identifier == upper_slc);
        let features = if includes_alias {
            vec![json!({
                "type": "Feature",
                "properties": {
                    "sceneName": MIXED_CASE_UPPER,
                    "url": "https://datapool.asf.alaska.edu/SLC/SA/mixed-case.zip",
                    "pathNumber": 88,
                    "frameNumber": 456
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [[[166,-45],[170,-45],[170,-35],[166,-35],[166,-45]]]
                }
            })]
        } else {
            Vec::new()
        };
        Ok(json!({"type": "FeatureCollection", "features": features}))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(Some(400))
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.cmr_calls.lock().unwrap().push(params.to_vec());
        let value = |name: &str| {
            params
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap()
        };
        assert_eq!(value("page_size"), "200");
        let page_number = value("page_num").parse::<u64>().unwrap();
        let bbox = value("bounding_box");
        let entries = match (bbox, page_number) {
            ("166,-47,180,-34", 1) => {
                std::iter::once(cmr_entry(MIXED_CASE_LOWER_ALIAS, 166.0, 179.0))
                    .chain(
                        (0..199).map(|index| cmr_entry(&generated_scene_id(index), 166.0, 179.0)),
                    )
                    .collect()
            }
            ("-180,-47,-175,-34", 1) => {
                std::iter::once(cmr_entry(MIXED_CASE_UPPER, -179.0, -175.0))
                    .chain(
                        (199..398)
                            .map(|index| cmr_entry(&generated_scene_id(index), -179.0, -175.0)),
                    )
                    .collect()
            }
            ("166,-47,180,-34", 2) => {
                vec![cmr_entry(&generated_scene_id(398), 166.0, 179.0)]
            }
            ("-180,-47,-175,-34", 2) => Vec::new(),
            _ => panic!("unexpected CMR request: bbox={bbox}, page={page_number}"),
        };
        Ok(json!({"feed": {"entry": entries}}))
    }
}

fn bounding_boxes(calls: &[Vec<(String, String)>]) -> Vec<&str> {
    calls
        .iter()
        .filter_map(|params| {
            params
                .iter()
                .find(|(name, _)| name == "bounding_box")
                .map(|(_, value)| value.as_str())
        })
        .collect()
}

#[test]
fn new_zealand_exact_query_never_falls_back_to_a_world_bbox() {
    let plan = plan_sar_query(&new_zealand_query()).unwrap();
    let wkt = plan.params.get("intersectsWith").expect("exact AOI WKT");

    assert!(wkt.starts_with("MULTIPOLYGON"));
    assert!(!wkt.contains("-179 -47, 179 -47"));
}

#[test]
fn new_zealand_single_cmr_param_helper_fails_closed_for_a_split_aoi() {
    assert!(
        cmr_page_params(&new_zealand_legacy_query(), 200, 1).is_none(),
        "one request cannot represent the logical union of two bounded CMR windows"
    );
}

#[test]
fn new_zealand_cmr_fallback_unions_two_bounded_windows_before_truncation() {
    const A: &str = "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_A001";
    const B: &str = "S1A_IW_SLC__1SDV_20240102T100000_20240102T100027_052001_064ABD_A002";
    const C: &str = "S1A_IW_SLC__1SDV_20240103T100000_20240103T100027_052002_064ABE_A003";
    let transport = CrossingCmrTransport::default();
    transport.cmr_responses.lock().unwrap().extend([
        json!({"feed": {"entry": [cmr_entry(A, 166.0, 179.0), cmr_entry(B, 170.0, 179.0)]}}),
        json!({"feed": {"entry": [cmr_entry(A, -179.0, -175.0), cmr_entry(C, -179.0, -175.0)]}}),
    ]);
    let provider = AsfProvider::with_transport(transport, "2026-08-19T00:00:00Z");

    let response = provider
        .search(
            &AsfRemoteSearchRequest {
                query: new_zealand_legacy_query(),
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .unwrap();

    let calls = provider.transport().cmr_calls.lock().unwrap();
    assert_eq!(
        bounding_boxes(&calls),
        ["166,-47,180,-34", "-180,-47,-175,-34"]
    );
    assert_eq!(
        response
            .scenes
            .iter()
            .map(|scene| scene.scene_id.as_str())
            .collect::<Vec<_>>(),
        [B, A, C],
        "higher coverage wins while equal A/C coverage keeps the first canonical occurrence order"
    );
    assert_eq!(response.stats.matched_count, None);
}

#[test]
fn new_zealand_cmr_keeps_page_size_fixed_until_the_true_second_page() {
    let provider =
        AsfProvider::with_transport(FixedPageCmrTransport::default(), "2026-08-19T00:00:00Z");
    let mut query = new_zealand_legacy_query();
    query.max_results = Some(100);

    let response = provider
        .search(
            &AsfRemoteSearchRequest {
                query,
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .expect("399 first-page canonical products must advance to the real second page");

    assert_eq!(response.stats.query_limit, 400);
    assert_eq!(response.scenes.len(), 100);
    assert_eq!(response.stats.matched_count, None);
    let calls = provider.transport().cmr_calls.lock().unwrap();
    let sequence = calls
        .iter()
        .map(|params| {
            let value = |name: &str| {
                params
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str())
                    .unwrap()
            };
            (
                value("page_size").to_string(),
                value("page_num").to_string(),
                value("bounding_box").to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sequence,
        [
            ("200".into(), "1".into(), "166,-47,180,-34".into()),
            ("200".into(), "1".into(), "-180,-47,-175,-34".into()),
            ("200".into(), "2".into(), "166,-47,180,-34".into()),
            ("200".into(), "2".into(), "-180,-47,-175,-34".into()),
        ]
    );
}

#[test]
fn mixed_case_sentinel_aliases_do_not_stop_cmr_before_the_true_next_page() {
    let provider = AsfProvider::with_transport(
        MixedCasePagedCmrTransport::default(),
        "2026-08-19T00:00:00Z",
    );
    let mut query = new_zealand_legacy_query();
    query.max_results = Some(400);

    let response = provider
        .search(
            &AsfRemoteSearchRequest {
                query,
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .unwrap();

    let calls = provider.transport().cmr_calls.lock().unwrap();
    let sequence = calls
        .iter()
        .map(|params| {
            let value = |name: &str| {
                params
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str())
                    .unwrap()
            };
            (
                value("page_size").to_string(),
                value("page_num").to_string(),
                value("bounding_box").to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sequence,
        [
            ("200".into(), "1".into(), "166,-47,180,-34".into()),
            ("200".into(), "1".into(), "-180,-47,-175,-34".into()),
            ("200".into(), "2".into(), "166,-47,180,-34".into()),
            ("200".into(), "2".into(), "-180,-47,-175,-34".into()),
        ]
    );
    drop(calls);

    assert_eq!(response.scenes.len(), 400);
    assert!(response
        .scenes
        .iter()
        .any(|scene| scene.scene_id == generated_scene_id(398)));
    let aliases = response
        .scenes
        .iter()
        .filter(|scene| scene.scene_id.to_ascii_uppercase() == MIXED_CASE_UPPER)
        .collect::<Vec<_>>();
    assert_eq!(aliases.len(), 1);
    let scene = aliases[0];
    assert_eq!(scene.scene_id, MIXED_CASE_LOWER_DISPLAY);
    assert_eq!(
        scene.url.as_deref(),
        Some("https://datapool.asf.alaska.edu/SLC/SA/mixed-case.zip")
    );
    assert_eq!(scene.path, Some(88));
    assert_eq!(scene.frame, Some(456));
    assert_eq!(
        scene
            .footprint_bbox
            .as_ref()
            .map(|bbox| (bbox.west, bbox.east)),
        Some((166.0, 170.0))
    );
    let enrichment_calls = provider.transport().enrichment_calls.lock().unwrap();
    assert!(enrichment_calls
        .iter()
        .any(|granule_list| granule_list.contains(MIXED_CASE_UPPER)));
    assert!(enrichment_calls
        .iter()
        .all(|granule_list| !granule_list.contains("064abc")));
}

#[test]
fn new_zealand_cmr_deduplicates_before_successful_enrichment() {
    const A: &str = "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_A001";
    const B: &str = "S1A_IW_SLC__1SDV_20240102T100000_20240102T100027_052001_064ABD_A002";
    const C: &str = "S1A_IW_SLC__1SDV_20240103T100000_20240103T100027_052002_064ABE_A003";
    let transport = CrossingCmrTransport::default();
    transport.asf_responses.lock().unwrap().extend([
        Err(scripted_asf_error("scripted primary failure")),
        Ok(json!({"type": "FeatureCollection", "features": [{
            "type": "Feature",
            "properties": {
                "sceneName": A,
                "url": "https://datapool.asf.alaska.edu/SLC/SA/enriched-a.zip",
                "pathNumber": 88,
                "frameNumber": 456
            },
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[166,-45],[170,-45],[170,-35],[166,-35],[166,-45]]]
            }
        }] })),
    ]);
    transport.cmr_responses.lock().unwrap().extend([
        json!({"feed": {"entry": [cmr_entry(A, 166.0, 179.0), cmr_entry(B, 170.0, 179.0)]}}),
        json!({"feed": {"entry": [cmr_entry(A, -179.0, -175.0), cmr_entry(C, -179.0, -175.0)]}}),
    ]);
    let provider = AsfProvider::with_transport(transport, "2026-08-19T00:00:00Z");

    let response = provider
        .search(
            &AsfRemoteSearchRequest {
                query: new_zealand_legacy_query(),
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .unwrap();

    assert_eq!(
        response
            .scenes
            .iter()
            .filter(|scene| scene.scene_id == A)
            .count(),
        1
    );
    let first = response
        .scenes
        .iter()
        .find(|scene| scene.scene_id == A)
        .expect("canonical first occurrence survives enrichment");
    assert_eq!(
        first.url.as_deref(),
        Some("https://datapool.asf.alaska.edu/SLC/SA/enriched-a.zip")
    );
    assert_eq!(first.path, Some(88));
    assert_eq!(first.frame, Some(456));
    assert_eq!(
        first
            .footprint_bbox
            .as_ref()
            .map(|bbox| (bbox.west, bbox.east)),
        Some((166.0, 170.0))
    );
}

#[test]
fn sentinel_aliases_dedupe_before_enrichment_and_path_frame_filtering() {
    const A: &str = "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_A001";
    let transport = CrossingCmrTransport::default();
    transport.asf_responses.lock().unwrap().extend([
        Err(scripted_asf_error("scripted primary failure")),
        Ok(json!({"type": "FeatureCollection", "features": [{
            "type": "Feature",
            "properties": {
                "sceneName": format!("https://datapool.asf.alaska.edu/SLC/SA/{A}.zip?alias=1"),
                "url": "https://datapool.asf.alaska.edu/SLC/SA/enriched-alias.zip",
                "pathNumber": 88,
                "frameNumber": 456
            },
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[166,-45],[170,-45],[170,-35],[166,-35],[166,-45]]]
            }
        }] })),
    ]);
    transport.cmr_responses.lock().unwrap().extend([
        json!({"feed": {"entry": [cmr_entry(&format!("{A}.SAFE"), 166.0, 179.0)]}}),
        json!({"feed": {"entry": [cmr_entry(A, -179.0, -175.0)]}}),
    ]);
    let provider = AsfProvider::with_transport(transport, "2026-08-19T00:00:00Z");
    let mut query = new_zealand_legacy_query();
    query.relative_orbit = Some("88".to_string());
    query.frame = Some("456".to_string());

    let response = provider
        .search(
            &AsfRemoteSearchRequest {
                query,
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .unwrap();

    assert_eq!(response.scenes.len(), 1);
    let scene = &response.scenes[0];
    assert_eq!(scene.scene_id, A);
    assert_eq!(
        scene.url.as_deref(),
        Some("https://datapool.asf.alaska.edu/SLC/SA/enriched-alias.zip")
    );
    assert_eq!(scene.path, Some(88));
    assert_eq!(scene.frame, Some(456));
    assert_eq!(
        scene
            .footprint_bbox
            .as_ref()
            .map(|bbox| (bbox.west, bbox.east)),
        Some((166.0, 170.0))
    );
}

#[test]
fn ambiguous_exact_aoi_cmr_fallback_fails_closed_before_remote_work() {
    let transport = CrossingCmrTransport::default();
    transport
        .cmr_responses
        .lock()
        .unwrap()
        .push_back(json!({"feed": {"entry": []}}));
    let provider = AsfProvider::with_transport(transport, "2026-08-19T00:00:00Z");
    let query = query_with(json!({
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[0,0],[120,0],[-120,1],[0,0]]]
        },
        "max_results": 20
    }));

    let error = provider
        .search(
            &AsfRemoteSearchRequest {
                query,
                allow_cmr_fallback: true,
            },
            &|| false,
        )
        .expect_err("an unprovable bbox projection must not become an unbounded CMR request");

    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(
        error.debug_context.get("reason").and_then(Value::as_str),
        Some("aoi_bbox_projection_unavailable")
    );
    assert!(provider.transport().cmr_calls.lock().unwrap().is_empty());
}

#[test]
fn count_and_refill_limits_preserve_explicit_all_results_and_safety_policy() {
    let mut unbounded = plan_asf_query(&query_with(json!({}))).expect("valid plan");
    assert_eq!(unbounded.requested_limit, 10_000);
    apply_remote_count(&mut unbounded, Some(640));
    assert_eq!(
        (unbounded.requested_limit, unbounded.query_limit),
        (640, 640)
    );
    assert_eq!(unbounded.params["maxResults"], "640");

    let mut bounded = plan_asf_query(&query_with(json!({
        "bbox": {"west": 0.0, "east": 1.0, "south": 0.0, "north": 1.0, "crs": "EPSG:4326"},
        "max_results": 60
    })))
    .expect("valid AOI plan");
    assert_eq!(bounded.aoi_mode, AoiPlanMode::Bbox);
    assert!(bounded.params["intersectsWith"].starts_with("POLYGON(("));
    assert_eq!(bounded.query_limit, 240);
    apply_remote_count(&mut bounded, Some(100));
    assert_eq!(bounded.query_limit, 100);
    assert_eq!(next_refill_limit(120, Some(500)), Some(240));
    assert_eq!(next_refill_limit(9_000, Some(9_500)), Some(9_500));
    assert_eq!(next_refill_limit(10_000, None), None);

    let mut complete_archive = plan_asf_query(&query_with(json!({}))).expect("valid plan");
    apply_remote_count(&mut complete_archive, Some(51_594));
    assert_eq!(complete_archive.requested_limit, 10_000);
    assert_eq!(complete_archive.query_limit, 10_000);
    assert_eq!(complete_archive.params["maxResults"], "10000");

    // Slice 3(a) deliberately replaces the historical unrestricted explicit target.
    let explicit_large = plan_asf_query(&query_with(json!({"max_results": 10_001})))
        .expect("explicit target is clamped to the shared safety ceiling");
    assert_eq!(explicit_large.requested_limit, 10_000);
    assert_eq!(explicit_large.query_limit, 10_000);
}

#[test]
fn complex_authoritative_aoi_remains_exact_for_remote_post_projection() {
    let polygons = (0..600)
        .map(|index| {
            let west = index as f64 * 0.001;
            let east = west + 0.0005;
            vec![vec![
                vec![west, 0.0],
                vec![east, 0.0],
                vec![east, 0.0005],
                vec![west, 0.0005],
                vec![west, 0.0],
            ]]
        })
        .collect::<Vec<_>>();
    let query = query_with(json!({
        "aoi_geojson": {"type": "MultiPolygon", "coordinates": polygons},
        "max_results": 10
    }));

    let plan = plan_asf_query(&query).expect("complex authoritative AOI remains searchable");

    assert_eq!(plan.aoi_mode, AoiPlanMode::Exact);
    let wkt = plan.params.get("intersectsWith").expect("exact AOI WKT");
    assert!(wkt.starts_with("MULTIPOLYGON ("));
    assert!(wkt.len() > 12_000);
    assert_eq!(
        query.aoi_geojson.as_ref().unwrap()["type"],
        "MultiPolygon",
        "the exact AOI remains on the immutable query for local filtering"
    );
}

#[test]
fn cmr_bbox_uses_only_validated_raw_aoi_or_raw_absent_bbox() {
    let invalid_raw = query_with(json!({
        "aoi_geojson": {"type": "Polygon", "coordinates": [[[0,0],[10,8],[0,10],[6,0],[0,0]]]},
        "bbox": {"west": 0.0, "east": 10.0, "south": 0.0, "north": 10.0, "crs": "EPSG:4326"}
    }));
    assert!(
        cmr_page_params(&invalid_raw, 10, 1).is_none(),
        "an invalid exact authority must fail closed instead of dropping the spatial constraint"
    );

    let valid_raw = query_with(json!({
        "aoi_geojson": {"type": "Polygon", "coordinates": [[[0,0],[1,0],[1,1],[0,1],[0,0]]]},
        "bbox": {"west": -5.0, "east": 5.0, "south": -5.0, "north": 5.0, "crs": "EPSG:4326"}
    }));
    let valid_params = cmr_page_params(&valid_raw, 10, 1).expect("SLC CMR route");
    assert_eq!(
        valid_params
            .iter()
            .find(|(name, _)| name == "bounding_box")
            .map(|(_, value)| value.as_str()),
        Some("0,0,1,1")
    );

    let bbox_only = query_with(json!({
        "bbox": {"west": 2.0, "east": 3.0, "south": 4.0, "north": 5.0, "crs": "EPSG:4326"}
    }));
    let bbox_only_params = cmr_page_params(&bbox_only, 10, 1).expect("SLC CMR route");
    assert_eq!(
        bbox_only_params
            .iter()
            .find(|(name, _)| name == "bounding_box")
            .map(|(_, value)| value.as_str()),
        Some("2,4,3,5")
    );
}

#[test]
fn invalid_dates_filters_and_short_windows_are_bounded_errors() {
    for query in [
        query_with(json!({"start": "2024-02-30"})),
        query_with(json!({"relative_orbit": "42;drop"})),
        query_with(json!({"product_type": "UNKNOWN"})),
    ] {
        let error = plan_asf_query(&query).expect_err("invalid query is rejected");
        assert_eq!(error.code, "ASF002");
        assert!(!error.to_string().contains("drop"));
    }
    assert!(split_query_windows(
        Some("2024-02-02"),
        Some("2024-02-01"),
        "2026-07-13T00:00:00Z"
    )
    .is_err());
    assert!(bisect_window(&QueryWindow {
        start: "2024-01-01T00:00:00Z".to_string(),
        end: "2024-01-02T00:00:00Z".to_string(),
    })
    .expect("valid window")
    .is_empty());
}

#[test]
fn legacy_sentinel_boundary_preserves_every_detailed_grd_level() {
    for level in ["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS", "GRD_FD"] {
        let plan = plan_asf_query(&query_with(json!({"product_type": level})))
            .unwrap_or_else(|error| panic!("{level}: {error}"));

        assert_eq!(
            plan.params.get("processingLevel").map(String::as_str),
            Some(level),
            "{level}"
        );
    }
}

#[test]
fn legacy_sentinel_boundary_rejects_descending_ranges() {
    for (field, value) in [("relative_orbit", "20-10"), ("frame", "010-002")] {
        let error = plan_asf_query(&query_with(json!({(field): value}))).unwrap_err();
        assert_eq!(error.code, "ASF002", "{field}");
        assert_eq!(
            error.user_message,
            format!(
                "ASF {} 参数范围起始值不能大于结束值",
                if field == "relative_orbit" {
                    "Path"
                } else {
                    "Frame"
                }
            ),
            "{field}"
        );
    }
}

#[test]
fn legacy_sentinel_boundary_validates_precise_grd_polarization() {
    for (level, invalid, valid) in [
        ("GRD_HD", "SV", "DV"),
        ("GRD_MD", "SH", "DH"),
        ("GRD_FD", "VV", "VV+VH"),
        ("GRD_HS", "DV", "SV"),
        ("GRD_MS", "HH+HV", "SH"),
    ] {
        let error = plan_asf_query(&query_with(json!({
            "product_type": level,
            "polarization": invalid
        })))
        .unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_INVALID", "{level}");
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some("polarizations"),
            "{level}"
        );

        plan_asf_query(&query_with(json!({
            "product_type": level,
            "polarization": valid
        })))
        .unwrap_or_else(|error| panic!("{level}: {error}"));
    }

    plan_asf_query(&query_with(json!({
        "product_type": "GRD_HD",
        "polarization": ""
    })))
    .expect("unspecified polarization remains valid");
}
