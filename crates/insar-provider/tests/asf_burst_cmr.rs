use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use insar_core::{
    AssetRole, ProviderResult, SarRemoteSearchRequest, SarSearchCountRequest, SarSearchQuery,
};
use insar_provider::asf::burst::{
    build_burst_cmr_query, map_burst_umm_granule, BurstCmrProvider, BurstSearchLimits,
    BURST_COLLECTION_CONCEPT_ID, BURST_PAGE_SIZE,
};
use insar_provider::asf::search::{
    AsfProvider, CmrUmmPage, MetadataTransport, SarSearchProviderProgress,
    SarSearchProviderProgressStage,
};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/asf/burst/sentinel-1-bursts-first-page.json");

#[derive(Default)]
struct ScriptedTransport {
    requests: Mutex<Vec<Vec<(String, String)>>>,
    responses: Mutex<VecDeque<ProviderResult<Value>>>,
    cancel_after_request: Option<Arc<AtomicBool>>,
}

impl ScriptedTransport {
    fn from_values(values: Vec<Value>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(values.into_iter().map(Ok).collect()),
            cancel_after_request: None,
        }
    }

    fn cancelling_after_first_request(values: Vec<Value>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(values.into_iter().map(Ok).collect()),
            cancel_after_request: Some(cancelled),
        }
    }
}

impl MetadataTransport for ScriptedTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Burst must not call SearchAPI GeoJSON")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("Burst must not call SearchAPI count")
    }

    fn cmr_json(
        &self,
        params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.requests.lock().unwrap().push(params.to_vec());
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted Burst CMR response");
        if let Some(cancelled) = &self.cancel_after_request {
            cancelled.store(true, Ordering::SeqCst);
        }
        response
    }
}

struct UmmOnlyTransport {
    response: Mutex<Option<Value>>,
    classic_calls: Arc<AtomicUsize>,
    umm_calls: Arc<AtomicUsize>,
}

impl MetadataTransport for UmmOnlyTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Burst must not call SearchAPI GeoJSON")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("Burst must not call SearchAPI count")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.classic_calls.fetch_add(1, Ordering::SeqCst);
        Err(insar_core::ContractError::new(
            "WRONG_CMR_ENDPOINT",
            "Burst selected the classic CMR JSON feed",
            serde_json::Map::new(),
        )
        .unwrap())
    }

    fn cmr_umm_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.umm_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .response
            .lock()
            .unwrap()
            .take()
            .expect("one UMM response"))
    }
}

struct SearchAfterTransport {
    requests: Mutex<Vec<Option<String>>>,
    request_params: Mutex<Vec<Vec<(String, String)>>>,
    responses: Mutex<VecDeque<CmrUmmPage>>,
}

impl SearchAfterTransport {
    fn from_pages(pages: Vec<CmrUmmPage>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            request_params: Mutex::new(Vec::new()),
            responses: Mutex::new(pages.into()),
        }
    }
}

impl MetadataTransport for SearchAfterTransport {
    fn asf_geojson(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Burst must not call SearchAPI GeoJSON")
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        panic!("Burst must not call SearchAPI count")
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("Search After transport must use the typed UMM page seam")
    }

    fn cmr_umm_page(
        &self,
        params: &[(String, String)],
        search_after: Option<&str>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CmrUmmPage> {
        self.request_params.lock().unwrap().push(params.to_vec());
        self.requests
            .lock()
            .unwrap()
            .push(search_after.map(str::to_owned));
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted Search After response"))
    }
}

fn raw_fixture() -> Value {
    serde_json::from_str(FIXTURE).expect("captured Burst fixture")
}

fn raw_item() -> Value {
    raw_fixture()["items"][0].clone()
}

fn complete_fixture() -> Value {
    let mut page = raw_fixture();
    page["hits"] = json!(page["items"].as_array().unwrap().len());
    page
}

fn full_page() -> Value {
    let mut page = raw_fixture();
    *page["items"].as_array_mut().unwrap() = vec![raw_item(); BURST_PAGE_SIZE as usize];
    page["hits"] = json!(BURST_PAGE_SIZE * 2);
    page
}

fn page(hits: u64, items: Vec<Value>) -> Value {
    json!({"hits": hits, "items": items})
}

fn transport_item(index: u64) -> Value {
    json!({"meta": {"native-id": format!("transport-item-{index}")}})
}

fn transport_page(hits: u64) -> Value {
    page(hits, (0..BURST_PAGE_SIZE).map(transport_item).collect())
}

fn distinct_raw_item(index: u64) -> Value {
    let mut item = raw_item();
    let native_id = format!("S1_301495_IW3_20141003T054235_VV_CASE{index:04}-BURST");
    item["meta"]["native-id"] = json!(native_id);
    item["meta"]["concept-id"] = json!(format!("G{index:010}-ASF"));
    item["umm"]["GranuleUR"] = json!(native_id);
    for related in item["umm"]["RelatedUrls"]
        .as_array_mut()
        .expect("fixture related URLs")
    {
        match related.get("Format").and_then(Value::as_str) {
            Some("GeoTIFF") => {
                related["URL"] = json!(format!(
                    "https://sentinel1-burst.asf.alaska.edu/test/{index}.tiff"
                ));
            }
            Some("XML") => {
                related["URL"] = json!(format!(
                    "https://sentinel1-burst.asf.alaska.edu/test/{index}.xml"
                ));
            }
            _ => {}
        }
    }
    item
}

fn set_attribute(item: &mut Value, name: &str, value: &str) {
    let attribute = item["umm"]["AdditionalAttributes"]
        .as_array_mut()
        .expect("fixture attributes")
        .iter_mut()
        .find(|attribute| attribute["Name"] == name)
        .unwrap_or_else(|| panic!("fixture attribute {name}"));
    attribute["Values"] = json!([value]);
}

fn limits(max_requests: u64, max_pages: u64, max_records: u64) -> BurstSearchLimits {
    BurstSearchLimits {
        max_requests,
        max_pages,
        max_records,
        max_raw_records: max_records,
        max_elapsed: Duration::from_secs(5),
    }
}

fn polygon() -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [-44.2, 48.4], [-42.8, 48.4], [-42.8, 49.0], [-44.2, 49.0], [-44.2, 48.4]
        ]]
    })
}

fn two_island_multipolygon() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[-44.2, 48.4], [-42.8, 48.4], [-42.8, 49.0], [-44.2, 49.0], [-44.2, 48.4]]],
            [[[10.0, 10.0], [11.0, 10.0], [11.0, 11.0], [10.0, 11.0], [10.0, 10.0]]]
        ]
    })
}

fn outside_aoi_item() -> Value {
    let mut item = raw_item();
    let native_id = "S1_301495_IW3_20141003T054235_VV_OUTSIDE-BURST";
    item["meta"]["native-id"] = json!(native_id);
    item["meta"]["concept-id"] = json!("G3128067031-ASF");
    item["umm"]["GranuleUR"] = json!(native_id);
    item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"][0]
        ["Boundary"]["Points"] = json!([
        { "Latitude": 30.0, "Longitude": -30.0 },
        { "Latitude": 30.5, "Longitude": -30.0 },
        { "Latitude": 30.5, "Longitude": -29.5 },
        { "Latitude": 30.0, "Longitude": -30.0 }
    ]);
    item
}

fn query(max_results: Option<u64>) -> SarSearchQuery {
    serde_json::from_value(json!({
        "provider": "ASF",
        "dataset": "SLC-BURST",
        "start": "2014-10-03T00:00:00Z",
        "end": "2014-10-04T00:00:00Z",
        "aoi_geojson": polygon(),
        "max_results": max_results,
        "burst": {
            "full_burst_id": "141_301495_IW3",
            "relative_burst_id": 301495,
            "absolute_burst_id": 5714273
        }
    }))
    .expect("valid Burst query")
}

#[test]
fn expands_desktop_date_only_values_to_the_full_utc_day() {
    let mut search = query(Some(3));
    search.start = Some("2026-05-26".to_owned());
    search.end = Some("2026-08-26".to_owned());

    let planned = build_burst_cmr_query(&search).expect("desktop dates plan for Burst CMR");
    let page = planned.page_params(&planned.polygons[0], 1, BURST_PAGE_SIZE);
    assert_eq!(
        params(&page, "temporal"),
        ["2026-05-26T00:00:00Z,2026-08-26T23:59:59Z"]
    );
}

#[test]
fn omits_temporal_for_the_desktop_all_time_choice() {
    let mut search = query(Some(100));
    search.start = None;
    search.end = None;

    let planned = build_burst_cmr_query(&search).expect("all-time Burst CMR query");
    let page = planned.page_params(&planned.polygons[0], 1, BURST_PAGE_SIZE);

    assert!(params(&page, "temporal").is_empty());
}

#[test]
fn rejects_a_half_open_desktop_date_range() {
    for (start, end, field) in [
        (Some("2026-08-01"), None, "end"),
        (None, Some("2026-08-26"), "start"),
    ] {
        let mut search = query(Some(100));
        search.start = start.map(str::to_owned);
        search.end = end.map(str::to_owned);

        let error = build_burst_cmr_query(&search).unwrap_err();

        assert_eq!(error.code, "ASF_QUERY_INVALID");
        assert_eq!(error.debug_context["field"], field);
        assert_eq!(error.debug_context["reason"], "paired_temporal");
    }
}

fn query_without_burst(max_results: Option<u64>) -> SarSearchQuery {
    let mut value = serde_json::to_value(query(max_results)).expect("Burst query JSON");
    value["burst"] = Value::Null;
    serde_json::from_value(value).expect("valid Burst query without IDs")
}

fn params<'a>(items: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    items
        .iter()
        .filter(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
        .collect()
}

#[test]
fn production_burst_route_selects_the_dedicated_cmr_umm_transport() {
    let classic_calls = Arc::new(AtomicUsize::new(0));
    let umm_calls = Arc::new(AtomicUsize::new(0));
    let transport = UmmOnlyTransport {
        response: Mutex::new(Some(complete_fixture())),
        classic_calls: Arc::clone(&classic_calls),
        umm_calls: Arc::clone(&umm_calls),
    };
    let provider = BurstCmrProvider::with_limits(&transport, limits(1, 1, 1)).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(1)),
            },
            &|| false,
            &|_| {},
        )
        .expect("Burst must consume one authoritative UMM page");

    assert_eq!(response.products.len(), 1);
    assert_eq!(classic_calls.load(Ordering::SeqCst), 0);
    assert_eq!(umm_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn translates_date_aoi_all_three_ids_and_finite_or_all_without_aliases() {
    let finite = build_burst_cmr_query(&query(Some(300))).unwrap();
    assert_eq!(finite.requested_limit, Some(300));
    assert_eq!(finite.safety_limit, 300);
    assert_eq!(finite.polygons.len(), 1);
    let page = finite.page_params(&finite.polygons[0], 1, BURST_PAGE_SIZE);
    assert_eq!(
        params(&page, "collection_concept_id"),
        [BURST_COLLECTION_CONCEPT_ID]
    );
    assert_eq!(
        params(&page, "temporal"),
        ["2014-10-03T00:00:00Z,2014-10-04T00:00:00Z"]
    );
    assert_eq!(params(&page, "page_size"), ["250"]);
    // #22: a search over its limit keeps the newest bursts, not the oldest.
    assert_eq!(params(&page, "sort_key"), ["-start_date"]);
    assert_eq!(
        params(&page, "attribute[]"),
        [
            "string,BURST_ID_FULL,141_301495_IW3",
            "int,BURST_ID_RELATIVE,301495",
            "int,BURST_ID_ABSOLUTE,5714273"
        ]
    );

    let all = build_burst_cmr_query(&query(None)).unwrap();
    assert_eq!(all.requested_limit, None);
    assert_eq!(all.safety_limit, 250);
}

#[test]
fn burst_explicit_limit_is_clamped_to_the_shared_ceiling() {
    for requested in [10_000, 10_001] {
        let plan = build_burst_cmr_query(&query(Some(requested))).unwrap();
        assert_eq!(plan.requested_limit, Some(10_000));
        assert_eq!(plan.safety_limit, 10_000);
    }
}

/// #22 B slice 3(a): generated Burst pages of `BURST_PAGE_SIZE` out of
/// `available`, all inside the AOI.
fn generated_burst_pages(available: u64) -> Vec<Value> {
    (0..available.div_ceil(BURST_PAGE_SIZE))
        .map(|page_index| {
            let start = page_index * BURST_PAGE_SIZE + 1;
            let end = (start + BURST_PAGE_SIZE - 1).min(available);
            page(available, (start..=end).map(distinct_raw_item).collect())
        })
        .collect()
}

#[test]
fn ten_thousand_target_keeps_n_and_flags_unproven_exhaustion() {
    for requested in [10_000, 10_001] {
        let transport = ScriptedTransport::from_values(generated_burst_pages(10_001));
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
        let response = provider
            .search(
                &SarRemoteSearchRequest {
                    query: query(Some(requested)),
                },
                &|| false,
                &|_| {},
            )
            .unwrap();
        assert_eq!(transport.requests.lock().unwrap().len(), 40);
        assert_eq!(response.products.len(), 10_000);
        assert_eq!(response.stats.requested_limit, 10_000);
        assert_eq!(response.stats.total_count, Some(10_001));
        assert_eq!(response.stats.truncated, Some(true));
    }
    let transport = ScriptedTransport::from_values(generated_burst_pages(10_000));
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(10_000)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(response.products.len(), 10_000);
    assert_eq!(response.stats.truncated, Some(false));
    assert!(response.stats.warnings.is_empty());
}

#[test]
fn translates_each_valid_burst_id_independently_without_aliases() {
    for (field, expected) in [
        ("full_burst_id", "string,BURST_ID_FULL,141_301495_IW3"),
        ("relative_burst_id", "int,BURST_ID_RELATIVE,301495"),
        ("absolute_burst_id", "int,BURST_ID_ABSOLUTE,5714273"),
    ] {
        let mut value = serde_json::to_value(query(Some(1))).unwrap();
        let burst = value["burst"].as_object_mut().unwrap();
        burst.retain(|key, _| key == field);
        let plan = build_burst_cmr_query(&serde_json::from_value(value).unwrap()).unwrap();
        let page = plan.page_params(&plan.polygons[0], 1, BURST_PAGE_SIZE);
        let attributes = params(&page, "attribute[]");
        assert_eq!(attributes, [expected]);
    }
}

#[test]
fn accepts_public_asf_burst_query_without_burst_ids() {
    let plan = build_burst_cmr_query(&query_without_burst(Some(1)))
        .expect("public ASF Burst query without IDs is valid");
    let page = plan.page_params(&plan.polygons[0], 1, BURST_PAGE_SIZE);

    assert!(params(&page, "attribute[]").is_empty());
}

#[test]
fn pushes_canonical_flight_direction_into_cmr_attributes() {
    let mut search = query_without_burst(Some(100));
    search.flight_direction = Some("descending".to_owned());

    let plan = build_burst_cmr_query(&search).expect("descending Burst query must be valid");
    let page = plan.page_params(&plan.polygons[0], 1, BURST_PAGE_SIZE);

    assert_eq!(
        params(&page, "attribute[]"),
        ["string,ASCENDING_DESCENDING,DESCENDING"]
    );
}

#[test]
fn default_burst_search_elapsed_budget_covers_complete_desktop_pagination() {
    let limits = BurstSearchLimits::default();

    assert_eq!(limits.max_requests, 512);
    assert_eq!(limits.max_pages, 512);
    assert_eq!(limits.max_records, 10_000);
    assert_eq!(limits.max_raw_records, 100_000);
    assert_eq!(limits.max_elapsed, Duration::from_secs(600));
}

#[test]
fn rejects_every_burst_query_field_that_the_formal_route_cannot_honor() {
    type ValueMutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, ValueMutation)> = vec![
        (
            "burst.burst_id",
            Box::new(|value| value["burst"]["burst_id"] = json!("legacy-id")),
        ),
        (
            "burst.subswath",
            Box::new(|value| value["burst"]["subswath"] = json!("IW3")),
        ),
        (
            "burst.burst_index",
            Box::new(|value| value["burst"]["burst_index"] = json!(0)),
        ),
        (
            "look_direction",
            Box::new(|value| value["look_direction"] = json!("RIGHT")),
        ),
        (
            "relative_orbit",
            Box::new(|value| value["relative_orbit"] = json!(139)),
        ),
        ("frame", Box::new(|value| value["frame"] = json!(1))),
        (
            "asf_frame",
            Box::new(|value| value["asf_frame"] = json!("1")),
        ),
        ("path", Box::new(|value| value["path"] = json!(139))),
        (
            "track_range",
            Box::new(|value| value["track_range"] = json!({"start": "1", "end": "2"})),
        ),
        (
            "frame_range",
            Box::new(|value| value["frame_range"] = json!({"start": "1", "end": "2"})),
        ),
        (
            "aoi_wkt",
            Box::new(|value| value["aoi_wkt"] = json!("POLYGON ((0 0, 1 0, 1 1, 0 0))")),
        ),
        (
            "bbox",
            Box::new(|value| {
                value["bbox"] = json!({
                    "west": -44.2,
                    "east": -42.8,
                    "south": 48.4,
                    "north": 49.0,
                    "crs": "EPSG:4326"
                })
            }),
        ),
    ];

    for (field, mutate) in cases {
        let mut value = serde_json::to_value(query(Some(1))).unwrap();
        mutate(&mut value);
        let query: SarSearchQuery = serde_json::from_value(value).unwrap();
        let error = build_burst_cmr_query(&query).unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED", "{field}");
        assert_eq!(error.debug_context["field"], field, "{field}");
    }
}

#[test]
fn validates_registry_backed_burst_filter_values_instead_of_ignoring_them() {
    type QueryMutation = Box<dyn Fn(&mut SarSearchQuery)>;
    let cases: Vec<(&str, QueryMutation)> = vec![
        (
            "platforms",
            Box::new(|query| query.platforms = vec!["Sentinel-X".to_owned()]),
        ),
        (
            "instrument",
            Box::new(|query| query.instrument = Some("NOT-C-SAR".to_owned())),
        ),
        (
            "processing_levels",
            Box::new(|query| query.processing_levels = vec!["L1".to_owned()]),
        ),
        (
            "product_families",
            Box::new(|query| query.product_families = vec!["SLC".to_owned()]),
        ),
        (
            "beam_modes",
            Box::new(|query| query.beam_modes = vec!["SM".to_owned()]),
        ),
        (
            "polarizations",
            Box::new(|query| query.polarizations = vec!["DV".to_owned()]),
        ),
        (
            "flight_direction",
            Box::new(|query| query.flight_direction = Some("SIDEWAYS".to_owned())),
        ),
    ];

    for (field, mutate) in cases {
        let mut search_query = query(Some(1));
        mutate(&mut search_query);
        let error = build_burst_cmr_query(&search_query).unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED", "{field}");
        assert_eq!(error.debug_context["field"], field, "{field}");
    }
}

#[test]
fn maps_official_identity_metadata_assets_and_explicit_preview_absence() {
    let product = map_burst_umm_granule(&raw_item()).unwrap();
    assert_eq!(
        product.product_key,
        "ASF::SLC-BURST::S1_301495_IW3_20141003T054235_VV_D5C8-BURST"
    );
    assert_eq!(
        product.scene_id.as_deref(),
        Some(product.product_id.as_str())
    );
    assert_eq!(product.platform.as_deref(), Some("SENTINEL-1A"));
    assert_eq!(product.instrument.as_deref(), Some("C-SAR"));
    assert_eq!(product.absolute_orbit, Some(2661));
    assert_eq!(product.relative_orbit, Some(139));
    assert_eq!(product.beam_swath.as_deref(), Some("IW3"));
    assert_eq!(
        product.burst.as_ref().unwrap().full_burst_id.as_deref(),
        Some("141_301495_IW3")
    );
    assert_eq!(product.burst.as_ref().unwrap().burst_index, Some(0));
    assert_eq!(product.native_metadata["preview_available"], false);
    assert_eq!(
        product.footprint_geojson,
        Some(
            json!({
                "type": "Polygon",
                "coordinates": [[
                    [-42.937422, 48.721505],
                    [-43.4852, 48.782833],
                    [-44.013133, 48.839436],
                    [-44.060795, 48.634682],
                    [-43.533918, 48.582698],
                    [-42.987175, 48.525997],
                    [-42.937422, 48.721505]
                ]]
            })
            .as_object()
            .unwrap()
            .clone()
        )
    );
    assert_eq!(product.assets.len(), 2);
    assert_eq!(product.assets[0].role, AssetRole::Primary);
    assert_eq!(product.assets[0].file_name.as_deref(), Some("0.tiff"));
    assert_eq!(product.assets[1].role, AssetRole::Metadata);
    assert_eq!(product.assets[1].file_name.as_deref(), Some("0.xml"));
}

#[test]
fn mapper_accepts_every_registry_sentinel_one_platform_with_c_sar() {
    for platform in ["SENTINEL-1A", "SENTINEL-1B", "SENTINEL-1C", "SENTINEL-1D"] {
        let mut item = raw_item();
        item["umm"]["Platforms"][0]["ShortName"] = json!(platform);

        let product = map_burst_umm_granule(&item)
            .unwrap_or_else(|error| panic!("registry platform {platform} must map: {error:?}"));

        assert_eq!(product.platform.as_deref(), Some(platform));
        assert_eq!(product.instrument.as_deref(), Some("C-SAR"));
    }
}

#[test]
fn freezes_safe_official_browse_relations_and_marks_preview_available() {
    let mut item = raw_item();
    item["umm"]["RelatedUrls"].as_array_mut().unwrap().extend([
        json!({
            "URL": "https://sentinel1-burst.asf.alaska.edu/browse/quick-look.png",
            "Type": "BROWSE"
        }),
        json!({
            "URL": "https://sentinel1-burst.asf.alaska.edu/browse/visualization.jpg",
            "Type": "GET RELATED VISUALIZATION"
        }),
    ]);

    let product = map_burst_umm_granule(&item).unwrap();

    assert_eq!(
        product.native_metadata["browse"]["urls"],
        json!([
            "https://sentinel1-burst.asf.alaska.edu/browse/quick-look.png",
            "https://sentinel1-burst.asf.alaska.edu/browse/visualization.jpg"
        ])
    );
    assert_eq!(product.native_metadata["preview_available"], true);
}

#[test]
fn ignores_unsafe_browse_relations_without_fabricating_preview() {
    for url in [
        "https://untrusted.example/browse.png",
        "https://sentinel1-burst.asf.alaska.edu/browse.png?token=secret",
        "https://user@sentinel1-burst.asf.alaska.edu/browse.png",
        "https://sentinel1-burst.asf.alaska.edu:444/browse.png",
    ] {
        let mut item = raw_item();
        item["umm"]["RelatedUrls"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "URL": url, "Type": "BROWSE" }));

        let product = map_burst_umm_granule(&item).unwrap();

        assert_eq!(product.native_metadata["browse"], Value::Null, "{url}");
        assert_eq!(product.native_metadata["preview_available"], false, "{url}");
    }
}

#[test]
fn preserves_authoritative_raw_cmr_umm_and_freezes_official_tiff_byte_length() {
    let item = raw_item();
    let product = map_burst_umm_granule(&item).unwrap();

    assert_eq!(product.file_size_remote, Some(171_218_848));
    assert_eq!(product.assets[0].size_bytes, Some(171_218_848));
    assert_eq!(product.assets[1].size_bytes, None);
    assert_eq!(
        product.native_metadata["raw_authority"]["meta"],
        item["meta"]
    );
    assert_eq!(product.native_metadata["raw_authority"]["umm"], item["umm"]);
}

#[test]
fn rejects_non_authoritative_burst_meta_provider_or_umm_format() {
    for (field, value) in [("provider-id", "OTHER"), ("format", "application/json")] {
        let mut item = raw_item();
        item["meta"][field] = json!(value);
        assert!(map_burst_umm_granule(&item).is_err(), "{field}");
    }
}

#[test]
fn preserves_every_official_burst_gpolygon_as_multipolygon() {
    let mut item = raw_item();
    let second = json!({
        "Boundary": {
            "Points": [
                { "Latitude": 10.0, "Longitude": 10.0 },
                { "Latitude": 10.0, "Longitude": 11.0 },
                { "Latitude": 11.0, "Longitude": 11.0 },
                { "Latitude": 11.0, "Longitude": 10.0 },
                { "Latitude": 10.0, "Longitude": 10.0 }
            ]
        }
    });
    item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"]
        .as_array_mut()
        .unwrap()
        .push(second);

    let product = map_burst_umm_granule(&item).unwrap();

    assert_eq!(
        product.footprint_geojson,
        Some(
            json!({
                "type": "MultiPolygon",
                "coordinates": [
                    [[
                        [-42.937422, 48.721505],
                        [-43.4852, 48.782833],
                        [-44.013133, 48.839436],
                        [-44.060795, 48.634682],
                        [-43.533918, 48.582698],
                        [-42.987175, 48.525997],
                        [-42.937422, 48.721505]
                    ]],
                    [[
                        [10.0, 10.0],
                        [11.0, 10.0],
                        [11.0, 11.0],
                        [10.0, 11.0],
                        [10.0, 10.0]
                    ]]
                ]
            })
            .as_object()
            .unwrap()
            .clone()
        )
    );
}

#[test]
fn invalid_burst_footprint_topology_fails_closed() {
    let cases = [
        (
            "self_intersection",
            json!([{
                "Boundary": { "Points": [
                    { "Latitude": 0.0, "Longitude": 0.0 },
                    { "Latitude": 1.0, "Longitude": 1.0 },
                    { "Latitude": 1.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 1.0 },
                    { "Latitude": 0.0, "Longitude": 0.0 }
                ] }
            }]),
        ),
        (
            "equal_area_backtracking_spike",
            json!([{
                "Boundary": { "Points": [
                    { "Latitude": 0.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 2.0 },
                    { "Latitude": 2.0, "Longitude": 2.0 },
                    { "Latitude": 2.0, "Longitude": 1.0 },
                    { "Latitude": 1.0, "Longitude": 1.0 },
                    { "Latitude": 2.0, "Longitude": 1.0 },
                    { "Latitude": 2.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 0.0 }
                ] }
            }]),
        ),
        (
            "zero_area_repeated_points",
            json!([{
                "Boundary": { "Points": [
                    { "Latitude": 0.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 1.0 },
                    { "Latitude": 0.0, "Longitude": 1.0 },
                    { "Latitude": 0.0, "Longitude": 0.0 }
                ] }
            }]),
        ),
        (
            "overlapping_multipolygon",
            json!([
                { "Boundary": { "Points": [
                    { "Latitude": 0.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 2.0 },
                    { "Latitude": 2.0, "Longitude": 2.0 },
                    { "Latitude": 2.0, "Longitude": 0.0 },
                    { "Latitude": 0.0, "Longitude": 0.0 }
                ] } },
                { "Boundary": { "Points": [
                    { "Latitude": 1.0, "Longitude": 1.0 },
                    { "Latitude": 1.0, "Longitude": 3.0 },
                    { "Latitude": 3.0, "Longitude": 3.0 },
                    { "Latitude": 3.0, "Longitude": 1.0 },
                    { "Latitude": 1.0, "Longitude": 1.0 }
                ] } }
            ]),
        ),
    ];

    for (name, polygons) in cases {
        let mut item = raw_item();
        item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"] = polygons;

        assert!(map_burst_umm_granule(&item).is_err(), "{name}");
    }
}

#[test]
fn rejects_burst_assets_using_a_nondefault_https_port() {
    let mut item = raw_item();
    item["umm"]["RelatedUrls"][0]["URL"] =
        json!("https://sentinel1-burst.asf.alaska.edu:444/path/0.tiff");

    assert!(map_burst_umm_granule(&item).is_err());
}

#[test]
fn maps_look_direction_from_current_umm_instrument_characteristic() {
    let mut item = raw_item();
    item["umm"]["Platforms"][0]["Instruments"][0]["Characteristics"][0]["Value"] = json!("LEFT");

    let product = map_burst_umm_granule(&item).unwrap();

    assert_eq!(product.look_direction.as_deref(), Some("LEFT"));
}

#[test]
fn rejects_duplicate_primary_burst_assets() {
    let mut item = raw_item();
    let primary = item["umm"]["RelatedUrls"][0].clone();
    *item["umm"]["RelatedUrls"].as_array_mut().unwrap() = vec![primary.clone(), primary];

    assert!(map_burst_umm_granule(&item).is_err());
}

#[test]
fn rejects_untrusted_or_noncanonical_burst_assets() {
    for url in [
        "http://sentinel1-burst.asf.alaska.edu/path/0.tiff",
        "https://untrusted.example/path/0.tiff",
        "https://sentinel1-burst.asf.alaska.edu/path/0.tiff?token=forbidden",
        "https://sentinel1-burst.asf.alaska.edu/path/0.xml",
        "https://sentinel1-burst.asf.alaska.edu/path/%30.tiff",
        "https://sentinel1-burst.asf.alaska.edu/path/0%20.tiff",
    ] {
        let mut item = raw_item();
        item["umm"]["RelatedUrls"][0]["URL"] = json!(url);
        assert!(map_burst_umm_granule(&item).is_err(), "{url}");
    }
}

#[test]
fn rejects_raw_whitespace_or_control_in_burst_asset_urls_before_url_parsing() {
    for url in [
        " https://sentinel1-burst.asf.alaska.edu/path/0.tiff",
        "\thttps://sentinel1-burst.asf.alaska.edu/path/0.tiff",
        "\nhttps://sentinel1-burst.asf.alaska.edu/path/0.tiff",
    ] {
        let mut item = raw_item();
        item["umm"]["RelatedUrls"][0]["URL"] = json!(url);

        assert!(map_burst_umm_granule(&item).is_err(), "{url:?}");
    }
}

#[test]
fn preserves_valid_footprint_above_core_projection_limit_within_burst_limit() {
    let mut item = raw_item();
    let mut points = (0..2_050)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / 2_050.0;
            json!({
                "Latitude": 10.0 + angle.sin(),
                "Longitude": 20.0 + angle.cos()
            })
        })
        .collect::<Vec<_>>();
    points.push(points[0].clone());
    let expected_coordinates = points
        .iter()
        .map(|point| json!([point["Longitude"], point["Latitude"]]))
        .collect::<Vec<_>>();
    item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"] =
        json!([{ "Boundary": { "Points": points } }]);

    let product = map_burst_umm_granule(&item).expect("valid bounded high-detail footprint");

    assert_eq!(
        product.footprint_geojson,
        Some(
            json!({ "type": "Polygon", "coordinates": [expected_coordinates] })
                .as_object()
                .unwrap()
                .clone()
        )
    );
}

#[test]
fn rejects_large_multipolygon_at_preflight_when_cross_component_work_exceeds_budget() {
    let mut item = raw_item();
    let polygons = (0..24)
        .map(|index| {
            let center_longitude = -150.0 + f64::from(index % 8) * 10.0;
            let center_latitude = -50.0 + f64::from(index / 8) * 10.0;
            let mut points = (0..200)
                .map(|point_index| {
                    let angle = std::f64::consts::TAU * f64::from(point_index) / 200.0;
                    json!({
                        "Latitude": center_latitude + 0.1 * angle.sin(),
                        "Longitude": center_longitude + 0.1 * angle.cos()
                    })
                })
                .collect::<Vec<_>>();
            points.push(points[0].clone());
            json!({ "Boundary": { "Points": points } })
        })
        .collect::<Vec<_>>();
    item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"] =
        Value::Array(polygons);

    let error = map_burst_umm_granule(&item).unwrap_err();

    assert_eq!(error.code, "ASF_PRODUCT_SCOPE_MISMATCH");
    assert_eq!(error.debug_context["field"], "footprint");
    assert_eq!(error.debug_context["reason"], "topology_work_ceiling");
}

#[test]
fn rejects_wrong_burst_collection_short_name_or_version() {
    for (key, value) in [("ShortName", "SENTINEL-1"), ("Version", "2")] {
        let mut item = raw_item();
        item["umm"]["CollectionReference"][key] = json!(value);
        assert!(map_burst_umm_granule(&item).is_err(), "{key}");
    }
}

#[test]
fn formal_provider_deduplicates_authoritative_burst_identity() {
    let mut page = raw_fixture();
    page["hits"] = json!(1);
    let transport = ScriptedTransport::from_values(vec![page]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(1)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap();
    assert_eq!(response.products.len(), 1);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn formal_post_filters_select_platform_beam_polarization_and_direction_and_fill_finite_n() {
    type QueryMutation = Box<dyn Fn(&mut SarSearchQuery)>;
    type ItemMutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, QueryMutation, ItemMutation)> = vec![
        (
            "platforms",
            Box::new(|query| query.platforms = vec!["sentinel-1b".to_owned()]),
            Box::new(|item| item["umm"]["Platforms"][0]["ShortName"] = json!("SENTINEL-1B")),
        ),
        (
            "beam_modes",
            Box::new(|query| query.beam_modes = vec!["ew".to_owned()]),
            Box::new(|item| {
                set_attribute(item, "BEAM_MODE", "EW");
                set_attribute(item, "SUBSWATH_NAME", "EW1");
            }),
        ),
        (
            "polarizations",
            Box::new(|query| query.polarizations = vec!["hh".to_owned()]),
            Box::new(|item| set_attribute(item, "POLARIZATION", "HH")),
        ),
        (
            "flight_direction",
            Box::new(|query| query.flight_direction = Some("ascending".to_owned())),
            Box::new(|item| set_attribute(item, "ASCENDING_DESCENDING", "ASCENDING")),
        ),
    ];

    for (name, mutate_query, mutate_item) in cases {
        let mut matching = distinct_raw_item(2);
        mutate_item(&mut matching);
        let expected_id = matching["meta"]["native-id"].as_str().unwrap().to_owned();
        let transport =
            ScriptedTransport::from_values(vec![page(2, vec![distinct_raw_item(1), matching])]);
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
        let progress = Mutex::new(Vec::new());
        let mut search_query = query(Some(1));
        mutate_query(&mut search_query);

        let response = provider
            .search(
                &SarRemoteSearchRequest {
                    query: search_query,
                },
                &|| false,
                &|value| progress.lock().unwrap().push(value),
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));

        assert_eq!(response.products.len(), 1, "{name}");
        assert_eq!(response.products[0].product_id, expected_id, "{name}");
        assert_eq!(response.stats.fetched_record_count, Some(2), "{name}");
        assert_eq!(response.stats.mapped_record_count, Some(2), "{name}");
        assert_eq!(response.stats.candidate_count, 1, "{name}");
        assert_eq!(response.stats.total_count, Some(2), "{name}");
        assert_eq!(response.stats.matched_count, Some(2), "{name}");
        assert!(
            progress
                .lock()
                .unwrap()
                .iter()
                .all(|value| value.remote_total.is_none() && value.total_count == Some(1)),
            "{name}"
        );
    }
}

#[test]
fn post_filtered_search_reports_the_provider_candidate_total_while_count_stays_unknown() {
    let mut matching = distinct_raw_item(2);
    matching["umm"]["Platforms"][0]["ShortName"] = json!("SENTINEL-1B");
    let mut filtered_query = query(None);
    filtered_query.platforms = vec!["Sentinel-1B".to_owned()];

    let search_transport =
        ScriptedTransport::from_values(vec![page(2, vec![distinct_raw_item(1), matching])]);
    let search_provider =
        BurstCmrProvider::with_limits(&search_transport, BurstSearchLimits::default()).unwrap();
    let progress = Mutex::new(Vec::new());
    let response = search_provider
        .search(
            &SarRemoteSearchRequest {
                query: filtered_query.clone(),
            },
            &|| false,
            &|value| progress.lock().unwrap().push(value),
        )
        .unwrap();

    assert_eq!(response.products.len(), 1);
    assert_eq!(response.stats.total_count, Some(2));
    assert_eq!(response.stats.matched_count, Some(2));
    assert!(progress
        .lock()
        .unwrap()
        .iter()
        .all(|value| value.remote_total.is_none() && value.total_count.is_none()));

    let count_transport = ScriptedTransport::from_values(vec![page(2, vec![raw_item()])]);
    let count_provider =
        BurstCmrProvider::with_limits(&count_transport, BurstSearchLimits::default()).unwrap();
    let count = count_provider
        .count(
            &SarSearchCountRequest {
                query: filtered_query,
            },
            &|| false,
        )
        .unwrap();
    assert_eq!(count.total_count, None);
}

#[test]
fn finite_post_filter_refills_across_pages_with_truthful_progress_and_stats() {
    let first_page = page(251, (1..=BURST_PAGE_SIZE).map(distinct_raw_item).collect());
    let mut matching = distinct_raw_item(251);
    matching["umm"]["Platforms"][0]["ShortName"] = json!("SENTINEL-1B");
    let expected_id = matching["meta"]["native-id"].as_str().unwrap().to_owned();
    let transport = ScriptedTransport::from_values(vec![first_page, page(251, vec![matching])]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let progress = Mutex::new(Vec::new());
    let mut search_query = query(Some(1));
    search_query.platforms = vec!["Sentinel-1B".to_owned()];

    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: search_query,
            },
            &|| false,
            &|value| progress.lock().unwrap().push(value),
        )
        .unwrap();

    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    assert_eq!(response.products.len(), 1);
    assert_eq!(response.products[0].product_id, expected_id);
    assert_eq!(response.stats.fetched_record_count, Some(251));
    assert_eq!(response.stats.mapped_record_count, Some(251));
    assert_eq!(response.stats.candidate_count, 1);
    assert_eq!(response.stats.total_count, Some(251));
    assert_eq!(response.stats.matched_count, Some(251));
    let progress = progress.lock().unwrap();
    assert_eq!(progress.len(), 4);
    assert!(progress
        .iter()
        .all(|value| value.remote_total.is_none() && value.total_count == Some(1)));
    assert_eq!(progress.last().unwrap().raw_fetched_count, 251);
    assert_eq!(progress.last().unwrap().mapped_count, 251);
    assert_eq!(progress.last().unwrap().unique_admitted_count, 1);
}

#[test]
fn finite_post_filter_continues_from_a_timed_out_short_page_with_search_after() {
    let first = CmrUmmPage {
        value: page(200, vec![distinct_raw_item(1), distinct_raw_item(2)]),
        search_after: Some("cursor-1".to_owned()),
        timed_out: true,
        search_after_capable: true,
    };
    let mut matching = distinct_raw_item(3);
    matching["umm"]["Platforms"][0]["ShortName"] = json!("SENTINEL-1B");
    let expected_id = matching["meta"]["native-id"].as_str().unwrap().to_owned();
    let second = CmrUmmPage {
        value: page(200, vec![matching]),
        search_after: Some("cursor-2".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![first, second]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let mut search_query = query(Some(1));
    search_query.platforms = vec!["Sentinel-1B".to_owned()];

    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: search_query,
            },
            &|| false,
            &|_| {},
        )
        .expect("Search After should continue from the returned cursor");

    assert_eq!(response.products[0].product_id, expected_id);
    assert_eq!(response.stats.metadata_request_count, Some(2));
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![None, Some("cursor-1".to_owned())]
    );
}

#[test]
fn continues_from_a_timed_out_empty_page_with_an_advancing_search_after_cursor() {
    let timed_out_empty = CmrUmmPage {
        value: page(1, Vec::new()),
        search_after: Some("cursor-1".to_owned()),
        timed_out: true,
        search_after_capable: true,
    };
    let final_page = CmrUmmPage {
        value: page(1, vec![distinct_raw_item(1)]),
        search_after: Some("cursor-final".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![timed_out_empty, final_page]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .expect("an advancing timeout cursor should resume a temporarily empty CMR page");

    assert_eq!(response.products.len(), 1);
    assert_eq!(response.stats.metadata_request_count, Some(2));
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![None, Some("cursor-1".to_owned())]
    );
}

#[test]
fn all_results_keep_the_search_after_page_size_constant_past_the_output_limit() {
    let first = CmrUmmPage {
        value: page(
            BURST_PAGE_SIZE + 1,
            (1..=BURST_PAGE_SIZE).map(distinct_raw_item).collect(),
        ),
        search_after: Some("cursor-1".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let second = CmrUmmPage {
        value: page(BURST_PAGE_SIZE + 1, vec![distinct_raw_item(251)]),
        search_after: Some("cursor-final".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![first, second]);
    let provider = BurstCmrProvider::with_limits(
        &transport,
        BurstSearchLimits {
            max_requests: 2,
            max_pages: 2,
            max_records: 10_000,
            max_raw_records: BURST_PAGE_SIZE + 1,
            max_elapsed: Duration::from_secs(5),
        },
    )
    .unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(BURST_PAGE_SIZE + 1)),
            },
            &|| false,
            &|_| {},
        )
        .expect("all-results search should exhaust the cursor with a stable page size");

    assert_eq!(response.stats.fetched_record_count, Some(251));
    assert_eq!(response.products.len(), 251);
    let requests = transport.request_params.lock().unwrap();
    assert_eq!(params(&requests[0], "page_size"), ["250"]);
    assert_eq!(params(&requests[1], "page_size"), ["250"]);
}

#[test]
fn search_after_walk_accepts_cmr_serving_fewer_items_than_hits() {
    // Live pattern captured from CMR for the SENTINEL-1_BURSTS collection over
    // a small AOI: page 1 serves fewer items than `hits` (156 of 180), the
    // follow-up cursor page is empty with no next cursor, and CMR keeps
    // reporting the same `hits`. The walk is complete, not truncated.
    let first = CmrUmmPage {
        value: page(180, (1..=3).map(distinct_raw_item).collect::<Vec<_>>()),
        search_after: Some("cursor-1".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let terminal = CmrUmmPage {
        value: page(180, Vec::new()),
        search_after: None,
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![first, terminal]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .expect("CMR may serve fewer Search-After items than the reported hits");

    assert_eq!(response.products.len(), 3);
    assert_eq!(response.stats.metadata_request_count, Some(2));
    assert_eq!(response.stats.fetched_record_count, Some(3));
    assert_eq!(response.stats.total_count, Some(180));
    // The walk finished (nothing left to request), but CMR served fewer items
    // than its reported hits. The batch stays possibly incomplete with a typed
    // warning instead of claiming a confirmed complete result (SAR-REG-008).
    assert_eq!(response.stats.truncated, Some(true));
    assert_eq!(response.stats.warnings.len(), 1);
    assert_eq!(
        response.stats.warnings[0].code,
        "ASF_SEARCH_POSSIBLY_INCOMPLETE"
    );
    assert_eq!(
        response.stats.warnings[0].debug_context["reason"],
        "served_fewer_than_hits"
    );
    assert_eq!(
        response.stats.warnings[0].debug_context["unserved_count"],
        serde_json::json!(177)
    );
    assert_eq!(
        *transport.requests.lock().unwrap(),
        vec![None, Some("cursor-1".to_owned())]
    );
}

#[test]
fn search_after_walk_stopping_short_of_hits_is_reported_as_possibly_incomplete() {
    // An upstream that stops early (here 3 of 180 with a cursor still offered,
    // there the terminal empty cursor-less page) must never be reported as a
    // confirmed complete result: the walk finishes only when the cursor ends,
    // and a shorter walk keeps truncated=true with a typed warning.
    let short_walk = CmrUmmPage {
        value: page(180, (1..=3).map(distinct_raw_item).collect::<Vec<_>>()),
        search_after: Some("cursor-1".to_owned()),
        timed_out: false,
        search_after_capable: true,
    };
    let terminal = CmrUmmPage {
        value: page(180, Vec::new()),
        search_after: None,
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![short_walk, terminal]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .expect("a walk that ends before the claimed hits still returns its products");

    assert_eq!(response.products.len(), 3);
    assert_eq!(response.stats.truncated, Some(true));
    assert_eq!(response.stats.warnings.len(), 1);
    assert_eq!(
        response.stats.warnings[0].code,
        "ASF_SEARCH_POSSIBLY_INCOMPLETE"
    );
    // An upstream that never offers another cursor on the very first page
    // while claiming hits is still a hard pagination failure.
    let stalled = CmrUmmPage {
        value: page(180, (1..=3).map(distinct_raw_item).collect::<Vec<_>>()),
        search_after: None,
        timed_out: false,
        search_after_capable: true,
    };
    let stalled_transport = SearchAfterTransport::from_pages(vec![stalled]);
    let stalled_provider =
        BurstCmrProvider::with_limits(&stalled_transport, BurstSearchLimits::default()).unwrap();
    let error = stalled_provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(error.debug_context["reason"], "missing_search_after");
    assert_eq!(stalled_transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn search_after_walk_rejects_an_empty_first_page_with_hits() {
    // An empty very first page while CMR still claims hits means the cursor
    // walk has produced nothing importable; that remains a pagination error.
    let first = CmrUmmPage {
        value: page(180, Vec::new()),
        search_after: None,
        timed_out: false,
        search_after_capable: true,
    };
    let transport = SearchAfterTransport::from_pages(vec![first]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let error = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .unwrap_err();

    assert_eq!(error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(error.debug_context["reason"], "empty_incomplete");
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn asf_provider_routes_canonical_burst_search_to_cmr_umm_only() {
    let transport = ScriptedTransport::from_values(vec![complete_fixture()]);
    let provider = AsfProvider::with_transport(transport, "2026-08-26T00:00:00Z");

    let response = provider
        .search_sar(
            &SarRemoteSearchRequest {
                query: query(Some(1)),
            },
            &|| false,
        )
        .expect("canonical Burst search reaches CMR UMM");

    assert_eq!(response.stats.source, "CMR_BURST");
    assert_eq!(provider.transport().requests.lock().unwrap().len(), 1);
}

#[test]
fn asf_provider_routes_canonical_burst_count_to_cmr_umm_only() {
    let transport = ScriptedTransport::from_values(vec![complete_fixture()]);
    let provider = AsfProvider::with_transport(transport, "2026-08-26T00:00:00Z");
    let request = SarSearchCountRequest {
        query: query(Some(1)),
    };

    let response = provider
        .count_sar(&request, &|| false)
        .expect("canonical Burst count reaches CMR UMM");

    assert_eq!(
        response.total_count,
        complete_fixture().get("hits").and_then(Value::as_u64)
    );
    assert_eq!(provider.transport().requests.lock().unwrap().len(), 1);
}

#[test]
fn multipolygon_unions_transport_parts_deduplicates_native_bursts_and_exactly_filters_aoi() {
    let mut search_query = query(Some(10));
    search_query.aoi_geojson = Some(two_island_multipolygon().as_object().unwrap().clone());
    let plan = build_burst_cmr_query(&search_query).unwrap();
    assert!(plan.polygons.len() > 1);

    let mut first_part = raw_fixture();
    first_part["hits"] = json!(2);
    *first_part["items"].as_array_mut().unwrap() = vec![raw_item(), outside_aoi_item()];
    let responses = std::iter::once(first_part)
        .chain((1..plan.polygons.len()).map(|_| complete_fixture()))
        .collect();
    let transport = ScriptedTransport::from_values(responses);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest {
                query: search_query,
            },
            &|| false,
            &|_| {},
        )
        .unwrap();

    assert_eq!(
        transport.requests.lock().unwrap().len(),
        plan.polygons.len()
    );
    assert_eq!(response.products.len(), 1);
    assert_eq!(
        response.products[0].product_id,
        "S1_301495_IW3_20141003T054235_VV_D5C8-BURST"
    );
}

#[test]
fn rejects_conflicting_authority_for_the_same_native_burst_identity() {
    let mut conflicting = raw_item();
    conflicting["meta"]["concept-id"] = json!("G9999999999-ASF");
    let mut page = complete_fixture();
    page["items"] = json!([raw_item(), conflicting]);
    page["hits"] = json!(2);
    let transport = ScriptedTransport::from_values(vec![page]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let error = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(10)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();

    assert_eq!(error.code, "ASF_IDENTITY_CONFLICT");
}

#[test]
fn malformed_burst_identity_scope_and_required_attributes_fail_closed() {
    type ItemMutation = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, ItemMutation)> = vec![
        (
            "native_id_mismatch",
            Box::new(|item| item["meta"]["native-id"] = json!("different-native-id")),
        ),
        (
            "wrong_collection",
            Box::new(|item| item["meta"]["collection-concept-id"] = json!("C0-ASF")),
        ),
        (
            "invalid_temporal",
            Box::new(|item| {
                item["umm"]["TemporalExtent"]["RangeDateTime"]["BeginningDateTime"] = json!("never")
            }),
        ),
        (
            "missing_orbit",
            Box::new(|item| item["umm"]["OrbitCalculatedSpatialDomains"] = json!([])),
        ),
        (
            "invalid_footprint",
            Box::new(|item| {
                item["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"] =
                    json!([])
            }),
        ),
        (
            "duplicate_required_attribute",
            Box::new(|item| {
                item["umm"]["AdditionalAttributes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({ "Name": "BURST_ID_FULL", "Values": ["141_301495_IW3"] }));
            }),
        ),
    ];
    for (name, mutate) in cases {
        let mut item = raw_item();
        mutate(&mut item);
        assert!(map_burst_umm_granule(&item).is_err(), "{name}");
    }
}

#[test]
fn rejects_repeated_non_progressing_burst_pages() {
    let page = full_page();
    let transport = ScriptedTransport::from_values(vec![page.clone(), page, json!({"items": []})]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    assert!(provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(3))
            },
            &|| false,
            &|_| {}
        )
        .is_err());
}

#[test]
fn rejects_short_burst_page_while_hits_remain() {
    let transport = ScriptedTransport::from_values(vec![raw_fixture()]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let error = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(3)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn rejects_changed_or_missing_hits_on_later_pages() {
    for (name, second_page, reason) in [
        ("changed", page(250, Vec::new()), "changed"),
        ("missing", json!({"items": []}), "missing"),
    ] {
        let transport = ScriptedTransport::from_values(vec![transport_page(251), second_page]);
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

        let error = provider
            .search(
                &SarRemoteSearchRequest {
                    query: query(Some(300)),
                },
                &|| false,
                &|_| {},
            )
            .unwrap_err();

        assert_eq!(error.code, "ASF_PAGINATION_INVALID", "{name}");
        assert_eq!(error.debug_context["field"], "hits", "{name}");
        assert_eq!(error.debug_context["reason"], reason, "{name}");
        assert_eq!(transport.requests.lock().unwrap().len(), 2, "{name}");
    }
}

#[test]
fn rejects_raw_fetched_count_exceeding_authoritative_hits() {
    let transport =
        ScriptedTransport::from_values(vec![page(1, vec![transport_item(1), transport_item(2)])]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let error = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(3)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();

    assert_eq!(error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(error.debug_context["field"], "fetched");
    assert_eq!(error.debug_context["reason"], "exceeds_hits");
}

#[test]
fn blank_result_limit_stops_at_the_bounded_presentation_page() {
    let transport = ScriptedTransport::from_values(vec![page(
        3,
        vec![distinct_raw_item(1), distinct_raw_item(2)],
    )]);
    let provider = BurstCmrProvider::with_limits(&transport, limits(4, 4, 2)).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .expect("blank quantity is a bounded presentation page");

    assert_eq!(response.products.len(), 2);
    assert_eq!(response.stats.total_count, Some(3));
    assert_eq!(response.stats.truncated, Some(true));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    assert_eq!(
        params(&transport.requests.lock().unwrap()[0], "page_size"),
        ["250"]
    );
}

#[test]
fn finite_and_all_progress_use_truthful_exact_counters_and_denominators() {
    let finite_transport = ScriptedTransport::from_values(vec![page(
        2,
        vec![distinct_raw_item(1), distinct_raw_item(2)],
    )]);
    let finite_provider =
        BurstCmrProvider::with_limits(&finite_transport, BurstSearchLimits::default()).unwrap();
    let finite_progress = Mutex::new(Vec::new());
    finite_provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(2)),
            },
            &|| false,
            &|value| finite_progress.lock().unwrap().push(value),
        )
        .unwrap();
    assert_eq!(
        *finite_progress.lock().unwrap(),
        vec![
            SarSearchProviderProgress {
                stage: SarSearchProviderProgressStage::ProviderResponse,
                provider_request_count: 1,
                transport_part_count: 1,
                raw_fetched_count: 2,
                mapped_count: 0,
                unique_admitted_count: 0,
                requested_limit: Some(2),
                remote_total: Some(2),
                fetched_count: 0,
                total_count: Some(2),
            },
            SarSearchProviderProgress {
                stage: SarSearchProviderProgressStage::AdmittingResults,
                provider_request_count: 1,
                transport_part_count: 1,
                raw_fetched_count: 2,
                mapped_count: 2,
                unique_admitted_count: 2,
                requested_limit: Some(2),
                remote_total: Some(2),
                fetched_count: 2,
                total_count: Some(2),
            },
        ]
    );

    let all_transport = ScriptedTransport::from_values(vec![page(
        2,
        vec![distinct_raw_item(3), distinct_raw_item(4)],
    )]);
    let all_provider =
        BurstCmrProvider::with_limits(&all_transport, BurstSearchLimits::default()).unwrap();
    let all_progress = Mutex::new(Vec::new());
    all_provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|value| all_progress.lock().unwrap().push(value),
        )
        .unwrap();
    assert_eq!(
        *all_progress.lock().unwrap(),
        vec![
            SarSearchProviderProgress {
                stage: SarSearchProviderProgressStage::ProviderResponse,
                provider_request_count: 1,
                transport_part_count: 1,
                raw_fetched_count: 2,
                mapped_count: 0,
                unique_admitted_count: 0,
                requested_limit: None,
                remote_total: Some(2),
                fetched_count: 0,
                total_count: Some(2),
            },
            SarSearchProviderProgress {
                stage: SarSearchProviderProgressStage::AdmittingResults,
                provider_request_count: 1,
                transport_part_count: 1,
                raw_fetched_count: 2,
                mapped_count: 2,
                unique_admitted_count: 2,
                requested_limit: None,
                remote_total: Some(2),
                fetched_count: 2,
                total_count: Some(2),
            },
        ]
    );
}

#[test]
fn finite_limit_stops_mapping_after_exact_admission_within_the_fetched_page() {
    let first = distinct_raw_item(1);
    let mut conflicting = first.clone();
    conflicting["meta"]["concept-id"] = json!("G9999999999-ASF");

    for (name, later_item) in [
        ("identity_conflict", conflicting),
        ("mapper_error", transport_item(2)),
    ] {
        let transport =
            ScriptedTransport::from_values(vec![page(2, vec![first.clone(), later_item])]);
        let provider =
            BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
        let progress = Mutex::new(Vec::new());

        let response = provider
            .search(
                &SarRemoteSearchRequest {
                    query: query(Some(1)),
                },
                &|| false,
                &|value| progress.lock().unwrap().push(value),
            )
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));

        assert_eq!(response.products.len(), 1, "{name}");
        assert!(response.errors.is_empty(), "{name}");
        assert_eq!(response.stats.fetched_record_count, Some(2), "{name}");
        assert_eq!(response.stats.mapped_record_count, Some(1), "{name}");
        assert_eq!(response.stats.mapping_error_count, Some(0), "{name}");
        let progress = progress.lock().unwrap();
        assert_eq!(progress.len(), 2, "{name}");
        assert_eq!(progress[1].unique_admitted_count, 1, "{name}");
        assert_eq!(progress[1].mapped_count, 1, "{name}");
        assert_eq!(progress[1].raw_fetched_count, 2, "{name}");
        assert_eq!(progress[1].fetched_count, 1, "{name}");
        assert_eq!(progress[1].total_count, Some(1), "{name}");
    }
}

#[test]
fn all_results_stats_report_the_actual_effective_page_size() {
    let transport = ScriptedTransport::from_values(vec![page(
        2,
        vec![distinct_raw_item(1), distinct_raw_item(2)],
    )]);
    let provider = BurstCmrProvider::with_limits(&transport, limits(4, 4, 2)).unwrap();

    let response = provider
        .search(
            &SarRemoteSearchRequest { query: query(None) },
            &|| false,
            &|_| {},
        )
        .unwrap();

    assert_eq!(response.stats.page_size, Some(BURST_PAGE_SIZE));
    assert_eq!(
        params(&transport.requests.lock().unwrap()[0], "page_size"),
        ["250"]
    );
}

#[test]
fn multipart_all_progress_never_claims_an_authoritative_remote_total() {
    let mut search_query = query(None);
    search_query.aoi_geojson = Some(two_island_multipolygon().as_object().unwrap().clone());
    let plan = build_burst_cmr_query(&search_query).unwrap();
    assert!(plan.polygons.len() > 1);
    let responses = (0..plan.polygons.len())
        .map(|index| page(1, vec![distinct_raw_item(index as u64 + 10)]))
        .collect();
    let transport = ScriptedTransport::from_values(responses);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let progress = Mutex::new(Vec::new());

    provider
        .search(
            &SarRemoteSearchRequest {
                query: search_query,
            },
            &|| false,
            &|value| progress.lock().unwrap().push(value),
        )
        .unwrap();

    assert!(progress
        .lock()
        .unwrap()
        .iter()
        .all(|value| value.remote_total.is_none() && value.total_count.is_none()));
}

#[test]
fn request_page_and_raw_record_ceilings_fail_closed() {
    let mut multipart_query = query(Some(10));
    multipart_query.aoi_geojson = Some(two_island_multipolygon().as_object().unwrap().clone());
    let request_transport = ScriptedTransport::from_values(vec![page(0, Vec::new())]);
    let request_provider =
        BurstCmrProvider::with_limits(&request_transport, limits(1, 4, 10)).unwrap();
    let request_error = request_provider
        .search(
            &SarRemoteSearchRequest {
                query: multipart_query,
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(request_error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(request_error.debug_context["reason"], "ceiling");

    let page_transport = ScriptedTransport::from_values(vec![transport_page(251)]);
    let page_provider = BurstCmrProvider::with_limits(&page_transport, limits(4, 1, 300)).unwrap();
    let page_error = page_provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(300)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(page_error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(page_error.debug_context["reason"], "ceiling");

    let raw_transport =
        ScriptedTransport::from_values(vec![page(2, vec![transport_item(1), transport_item(2)])]);
    let raw_provider = BurstCmrProvider::with_limits(&raw_transport, limits(2, 2, 1)).unwrap();
    let raw_error = raw_provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(1)),
            },
            &|| false,
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(raw_error.code, "ASF_PAGINATION_INVALID");
    assert_eq!(raw_error.debug_context["field"], "records");
    assert_eq!(raw_error.debug_context["reason"], "ceiling");
}

#[test]
fn stops_when_cancelled_immediately_after_burst_transport() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let transport = ScriptedTransport::cancelling_after_first_request(
        vec![raw_fixture()],
        Arc::clone(&cancelled),
    );
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();

    let error = provider
        .search(
            &SarRemoteSearchRequest {
                query: query(Some(1)),
            },
            &|| cancelled.load(Ordering::SeqCst),
            &|_| {},
        )
        .unwrap_err();
    assert_eq!(error.code, "DL001");
}

/// Recorded 2026-10-20T03:30Z from the anonymous CMR UMM-JSON granule
/// search the desktop uses (collection C2709161906-ASF, temporal
/// 2026-08-01T00:00:00Z..2026-10-19T23:59:59.999Z, the AOI W105.4930
/// E105.6289 S24.6707 N24.7880; three pages of 20 concatenated). CMR reported
/// 44 hits; every granule is a Sentinel-1D burst. The desktop mapped none of
/// them (Owner desktop recheck: matched 44, fetched 44, mapped 0).
const RECORDED_SENTINEL_1D: &str =
    include_str!("fixtures/asf/burst/sentinel-1-bursts-recorded-2026-10-20.json");

#[test]
fn recorded_sentinel_1d_bursts_map_with_their_scientific_identity() {
    let fixture: Value = serde_json::from_str(RECORDED_SENTINEL_1D).unwrap();
    let items = fixture["items"].as_array().unwrap();
    assert_eq!(fixture["hits"], json!(44));
    assert_eq!(items.len(), 44);
    let mut errors = BTreeMap::new();
    let mut mapped = Vec::new();
    for item in items {
        match map_burst_umm_granule(item) {
            Ok(product) => mapped.push(product),
            Err(error) => *errors.entry(format!("{error:?}")).or_insert(0_u32) += 1,
        }
    }
    assert!(errors.is_empty(), "rejected granules: {errors:#?}");
    assert_eq!(mapped.len(), 44);
    for product in &mapped {
        assert_eq!(product.platform.as_deref(), Some("SENTINEL-1D"));
        assert_eq!(product.dataset, "SLC-BURST");
        let burst = product.burst.as_ref().unwrap();
        assert!(burst
            .full_burst_id
            .as_deref()
            .is_some_and(|id| !id.is_empty()));
        assert!(burst.relative_burst_id.is_some_and(|id| id > 0));
        assert!(matches!(product.polarization.as_deref(), Some("VV" | "VH")));
        assert!(product.absolute_orbit.is_some_and(|orbit| orbit > 0));
        assert!(product.acquisition_start.is_some());
        assert!(product.footprint_geojson.is_some());
    }
}

/// The recorded Sentinel-1D page reaches the search results through the
/// production provider: CMR's 44 hits become 44 candidates for the AOI.
#[test]
fn recorded_sentinel_1d_bursts_reach_the_search_results() {
    let fixture: Value = serde_json::from_str(RECORDED_SENTINEL_1D).unwrap();
    let transport = ScriptedTransport::from_values(vec![fixture]);
    let provider = BurstCmrProvider::with_limits(&transport, BurstSearchLimits::default()).unwrap();
    let query: SarSearchQuery = serde_json::from_value(json!({
        "provider": "ASF",
        "dataset": "SLC-BURST",
        "start": "2026-08-01",
        "end": "2026-10-19",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[
                [105.4930, 24.6707], [105.6289, 24.6707], [105.6289, 24.7880],
                [105.4930, 24.7880], [105.4930, 24.6707]
            ]]
        },
        "max_results": null
    }))
    .unwrap();
    let response = provider
        .search(&SarRemoteSearchRequest { query }, &|| false, &|_| {})
        .unwrap();
    assert_eq!(response.products.len(), 44);
    assert!(response
        .products
        .iter()
        .all(|product| product.platform.as_deref() == Some("SENTINEL-1D")));
}
