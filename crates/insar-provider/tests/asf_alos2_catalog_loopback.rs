use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use insar_core::CatalogSearchQuery;
use insar_provider::asf::catalog_client::{
    AsfCatalogClient, CatalogSearchLimits, CatalogSearchProviderProgress,
};
use insar_provider::asf::client::{AsfHttpClient, HttpLimits, RemoteEndpoints};
use insar_provider::asf::query::build_alos2_cmr_query;
use insar_provider::catalog::{
    CollectionProfileRegistry, DatasetRoutePolicyRegistry, ALOS2_LOGICAL_DATASET,
    ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID, ALOS2_ROUTE_ID,
};
use serde_json::{json, Value};

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
}

impl Reply {
    fn page(hits: u64, items: Vec<Value>) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&json!({"hits": hits, "items": items})).unwrap(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

struct LoopbackServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackServer {
    fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("loopback address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = loop {
                    if worker_stop.load(Ordering::Acquire) {
                        return;
                    }
                    match listener.accept() {
                        Ok(stream) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("accept loopback request: {error}"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking loopback request stream");
                let request = read_request(&mut stream);
                captured
                    .lock()
                    .expect("request lock")
                    .push(String::from_utf8_lossy(&request).into_owned());
                thread::sleep(reply.delay);
                let reason = if reply.status >= 500 {
                    "Server Error"
                } else if reply.status >= 400 {
                    "Client Error"
                } else {
                    "OK"
                };
                let header = format!(
                    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reason,
                    reply.body.len()
                );
                if stream.write_all(header.as_bytes()).is_ok() {
                    for chunk in reply.body.chunks(16 * 1024) {
                        if stream.write_all(chunk).is_err() {
                            break;
                        }
                        let _ = stream.flush();
                    }
                }
            }
        });
        Self {
            url: format!("http://{address}/search/granules.umm_json"),
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("request lock").clone()
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request lock").len()
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().expect("loopback worker");
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("read timeout");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap_or(0);
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    request
}

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("ALOS-2 fixture exists"))
        .expect("ALOS-2 fixture is valid JSON")
}

fn granule(index: usize) -> Value {
    let mut value = fixture("granule-wbd-cmr-umm.json");
    let concept_id = format!("G9{index:09}-ASF");
    let native_id = format!("ALOS2-CATALOG-{index:06}");
    let file_name = format!("ALOS2-CATALOG-{index:06}.zip");
    value["meta"]["concept-id"] = json!(concept_id);
    value["meta"]["native-id"] = json!(native_id);
    value["umm"]["GranuleUR"] = json!(native_id);
    value["umm"]["TemporalExtent"]["RangeDateTime"]["BeginningDateTime"] =
        json!("2026-01-01T01:02:03Z");
    value["umm"]["TemporalExtent"]["RangeDateTime"]["EndingDateTime"] =
        json!("2026-01-01T01:02:55Z");
    value["umm"]["DataGranule"]["Identifiers"][0]["Identifier"] =
        json!(format!("ALOS2-{index:06}"));
    value["umm"]["DataGranule"]["ArchiveAndDistributionInformation"][0]["Name"] = json!(file_name);
    value["umm"]["RelatedUrls"][0]["URL"] =
        json!(format!("https://example.invalid/alos2/{file_name}"));
    value["umm"]["RelatedUrls"][1]["URL"] =
        json!(format!("s3://example-invalid/alos2/{file_name}"));
    value
}

fn granule_with_footprint(index: usize, west: f64, south: f64, east: f64, north: f64) -> Value {
    let mut value = granule(index);
    value["umm"]["SpatialExtent"]["HorizontalSpatialDomain"]["Geometry"]["GPolygons"][0]
        ["Boundary"]["Points"] = json!([
        {"Latitude": south, "Longitude": west},
        {"Latitude": south, "Longitude": east},
        {"Latitude": north, "Longitude": east},
        {"Latitude": north, "Longitude": west},
        {"Latitude": south, "Longitude": west}
    ]);
    value
}

fn granule_with_beam_mode(index: usize, beam_mode: &str) -> Value {
    let mut value = granule(index);
    value["umm"]["AdditionalAttributes"][1]["Values"][0] = json!(beam_mode);
    value
}

fn polygon_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": ALOS2_LOGICAL_DATASET,
        "provider_id": ALOS2_PROVIDER_ID,
        "native_collection_id": ALOS2_NATIVE_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [113.0, 30.0], [113.0, 33.0], [110.0, 33.0], [110.0, 30.0]]]
        },
        "cursor": null,
        "result_limit": 2,
        "filters": {
            "datetime": {"start":"2026-01-01T00:00:00Z", "end":"2026-01-02T00:00:00Z"},
            "beam_mode": "WBD",
            "polarizations": ["HH", "HV"],
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT",
            "path": 161,
            "frame": 3150
        }
    }))
    .expect("closed ALOS-2 catalogue query")
}

fn client(server: &LoopbackServer, limits: CatalogSearchLimits) -> AsfCatalogClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    let http = AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default())
        .expect("loopback HTTP client");
    AsfCatalogClient::with_http_client_for_tests(http, limits).expect("bounded catalogue client")
}

fn search(
    client: &AsfCatalogClient,
    query: &CatalogSearchQuery,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<insar_core::CatalogSearchResponse, insar_core::ContractError> {
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");
    client.search_alos2(query, profile, cancelled)
}

#[test]
fn production_catalogue_route_is_enabled_before_network_access() {
    let route = DatasetRoutePolicyRegistry
        .for_logical_dataset(ALOS2_LOGICAL_DATASET)
        .expect("ALOS-2 route is registered");
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 collection profile is registered");

    assert_eq!(route.route_id, ALOS2_ROUTE_ID);
    assert!(route.production_enabled);
    assert!(profile.production_enabled);
}

#[test]
fn finite_structured_query_stops_after_the_requested_working_set() {
    let server = LoopbackServer::start(vec![Reply::page(4, vec![granule(1), granule(2)])]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("bounded working-set search");

    assert_eq!(
        response.collection.native_collection_id,
        ALOS2_NATIVE_COLLECTION_ID
    );
    assert_eq!(response.stats.matched_count, Some(4));
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.mapped_record_count, 2);
    assert_eq!(response.stats.exact_duplicate_record_count, 0);
    assert_eq!(response.stats.unique_provider_product_count, 2);
    assert_eq!(response.stats.returned_count, 2);
    assert_eq!(response.stats.requested_limit, Some(2));
    assert_eq!(response.stats.request_count, 1);
    assert_eq!(response.stats.transport_part_count, 1);
    assert_eq!(response.stats.exact_matched_count, 2);
    assert!(!response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    assert!(response.next_cursor.is_none());
    assert!(response.item_errors.is_empty());
    let raw_mapped = response
        .products
        .iter()
        .find(|product| product.product_id == "G9000000001-ASF")
        .expect("raw CMR UMM item is normalized by granule concept ID");
    assert_eq!(raw_mapped.display_name, "ALOS2-CATALOG-000001");
    assert_eq!(raw_mapped.properties["path"], 161);
    assert_eq!(raw_mapped.properties["frame"], 3150);
    assert!(raw_mapped.footprint_geojson.is_some());
    let asset_key = raw_mapped.assets[0].asset_key.as_deref().unwrap();
    let routes = raw_mapped.properties["asset_routes"][asset_key]
        .as_array()
        .unwrap();
    assert_eq!(routes.len(), 2, "the credentials endpoint is not an asset");
    assert!(routes
        .iter()
        .all(|route| !route["locator"].as_str().unwrap().contains("credentials")));

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("GET /search/granules.umm_json?"),
        "{}",
        requests[0]
    );
    for (index, request) in requests.iter().enumerate() {
        assert!(request.contains("collection_concept_id=C3315903479-ASF"));
        assert!(request.contains("page_size=2"));
        assert!(request.contains(&format!("page_num={}", index + 1)));
        assert!(request.contains("temporal=2026-01-01T00%3A00%3A00Z%2C2026-01-02T00%3A00%3A00Z"));
        assert!(request.contains("attribute%5B%5D=string%2CBEAM_MODE%2CWBD"));
        assert!(request.contains("attribute%5B%5D=string%2CPOLARIZATION%2CHH"));
        assert!(request.contains("attribute%5B%5D=string%2CPOLARIZATION%2CHV"));
        assert!(request.contains("attribute%5B%5D=int%2CPATH_NUMBER%2C161"));
        assert!(request.contains("attribute%5B%5D=int%2CFRAME_NUMBER%2C3150"));
        assert!(request.contains("polygon="));
    }
}

#[test]
fn multipolygon_uses_one_bounded_envelope_and_exact_candidate_filtering() {
    let mut query = polygon_query();
    query.result_limit = Some(5);
    query.aoi_geojson = Some(
        json!({
            "type":"MultiPolygon",
            "coordinates":[
                [[[120.0, 20.0], [121.0, 20.0], [121.0, 21.0], [120.0, 21.0], [120.0, 20.0]]],
                [[[100.0, 10.0], [101.0, 10.0], [101.0, 11.0], [100.0, 11.0], [100.0, 10.0]]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let server = LoopbackServer::start(vec![Reply::page(
        1,
        vec![granule_with_footprint(10, 100.2, 10.2, 100.8, 10.8)],
    )]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("bounded MultiPolygon search");

    assert_eq!(response.stats.matched_count, Some(1));
    assert_eq!(response.stats.fetched_record_count, 1);
    assert_eq!(response.stats.exact_duplicate_record_count, 0);
    assert_eq!(response.stats.unique_provider_product_count, 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("polygon=100"), "{}", requests[0]);
    assert_eq!(
        response.products[0].properties["spatial_transport_mode"],
        "cmr_conservative_bbox_exact_post_filter"
    );
}

#[test]
fn polygon_hole_uses_conservative_exterior_transport_and_exact_candidate_filtering() {
    let mut query = polygon_query();
    query.result_limit = Some(1);
    query.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [
                [[-114.0,21.0],[-109.0,21.0],[-109.0,26.0],[-114.0,26.0],[-114.0,21.0]],
                [[-112.5,22.0],[-110.0,22.0],[-110.0,24.5],[-112.5,24.5],[-112.5,22.0]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let inside_hole = granule_with_footprint(13, -112.0, 22.5, -110.5, 24.0);
    let intersects_shell = granule_with_footprint(14, -113.8, 21.2, -113.0, 22.0);
    let server = LoopbackServer::start(vec![
        Reply::page(2, vec![inside_hole]),
        Reply::page(2, vec![intersects_shell]),
    ]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("hole AOI uses a conservative provider query plus exact local geometry filter");

    assert_eq!(response.stats.matched_count, Some(2));
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.products.len(), 1);
    assert_eq!(response.products[0].product_id, "G9000000014-ASF");
    assert_eq!(
        response.products[0].properties["spatial_transport_mode"],
        "cmr_conservative_bbox_exact_post_filter"
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("polygon=-114"), "{}", requests[0]);
    assert!(
        requests.iter().all(|request| request.len() < 8_000),
        "hole geometry transport must remain provider-safe"
    );
}

#[test]
fn geojson_feature_wrappers_preserve_complete_part_semantics() {
    let mut feature_query = polygon_query();
    let geometry = json!({
        "type":"Polygon",
        "coordinates":[[[110.0,30.0,999.0],[111.0,30.0,999.0],[111.0,31.0,999.0],[110.0,30.0,999.0]]]
    });
    feature_query.aoi_geojson = Some(
        json!({
            "type": "Feature",
            "properties": {},
            "geometry": geometry
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let server = LoopbackServer::start(vec![Reply::page(1, vec![granule(11)])]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &feature_query,
        &|| false,
    )
    .expect("a Polygon Feature remains one complete provider query");
    assert_eq!(response.stats.matched_count, Some(1));
    let requests = server.requests();
    let request_line = requests[0]
        .lines()
        .next()
        .expect("captured HTTP request line");
    assert!(
        !request_line.contains("999"),
        "provider query must discard GeoJSON Z coordinates: {request_line}"
    );
    drop(server);

    let mut collection_query = polygon_query();
    collection_query.aoi_geojson = Some(json!({
        "type":"FeatureCollection",
        "features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[120.0,20.0],[121.0,20.0],[121.0,21.0],[120.0,21.0],[120.0,20.0]]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[100.0,10.0],[101.0,10.0],[101.0,11.0],[100.0,11.0],[100.0,10.0]]]}}
        ]
    })
    .as_object()
    .unwrap()
    .clone());
    let server = LoopbackServer::start(vec![Reply::page(
        1,
        vec![granule_with_footprint(12, 100.2, 10.2, 100.8, 10.8)],
    )]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &collection_query,
        &|| false,
    )
    .expect("FeatureCollection uses bounded conservative transport");
    assert_eq!(response.stats.matched_count, Some(1));
    assert_eq!(response.stats.fetched_record_count, 1);
    assert_eq!(response.stats.unique_provider_product_count, 1);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn incoming_cursor_and_repeated_pages_fail_without_partial_publication() {
    let idle = LoopbackServer::start(vec![Reply::page(0, vec![])]);
    let mut continuation = polygon_query();
    continuation.cursor = Some("caller-page-2".to_string());
    let error = search(
        &client(&idle, CatalogSearchLimits::default()),
        &continuation,
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_CURSOR_UNSUPPORTED");
    assert_eq!(idle.request_count(), 0);
    drop(idle);

    let repeated = granule(20);
    let mut replayed_with_changed_metadata = repeated.clone();
    replayed_with_changed_metadata["meta"]["revision-id"] = json!(2);
    replayed_with_changed_metadata["umm"]["RelatedUrls"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let server = LoopbackServer::start(vec![
        Reply::page(4, vec![repeated.clone(), repeated]),
        Reply::page(
            4,
            vec![
                replayed_with_changed_metadata.clone(),
                replayed_with_changed_metadata,
            ],
        ),
    ]);
    let mut repeated_query = polygon_query();
    repeated_query.result_limit = Some(2);
    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &repeated_query,
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_PAGINATION_INVALID");
    assert_eq!(server.request_count(), 2);
}

#[test]
fn request_page_record_and_progress_budgets_fail_closed_or_complete_honestly() {
    let duplicate = granule(30);
    let page_server =
        LoopbackServer::start(vec![Reply::page(3, vec![duplicate.clone(), duplicate])]);
    let error = search(
        &client(
            &page_server,
            CatalogSearchLimits {
                max_pages: 1,
                ..CatalogSearchLimits::default()
            },
        ),
        &polygon_query(),
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_PAGE_BUDGET_EXCEEDED");
    assert_eq!(page_server.request_count(), 1);

    let mut multipolygon = polygon_query();
    multipolygon.aoi_geojson = Some(
        json!({
            "type":"Polygon",
            "coordinates":[
                [[80.0,10.0],[140.0,10.0],[140.0,50.0],[80.0,50.0],[80.0,10.0]],
                [[85.0,15.0],[95.0,15.0],[95.0,25.0],[85.0,25.0],[85.0,15.0]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");
    let plan = build_alos2_cmr_query(&multipolygon, profile).expect("multi-part request plan");
    assert!(plan.transport_part_count > 1);
    let request_server = LoopbackServer::start(vec![Reply::page(0, vec![])]);
    let error = search(
        &client(
            &request_server,
            CatalogSearchLimits {
                max_requests: 1,
                ..CatalogSearchLimits::default()
            },
        ),
        &multipolygon,
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_REQUEST_BUDGET_EXCEEDED");
    assert_eq!(request_server.request_count(), 1);

    let record_server = LoopbackServer::start(vec![Reply::page(2, vec![granule(40)])]);
    let error = search(
        &client(
            &record_server,
            CatalogSearchLimits {
                max_records: 1,
                ..CatalogSearchLimits::default()
            },
        ),
        &polygon_query(),
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_WORKING_SET_INCOMPLETE");
    assert_eq!(record_server.request_count(), 1);

    let time_server = LoopbackServer::start(vec![
        Reply::page(0, vec![]).delayed(Duration::from_millis(30))
    ]);
    let response = search(
        &client(
            &time_server,
            CatalogSearchLimits {
                max_progress_idle: Duration::from_millis(5),
                ..CatalogSearchLimits::default()
            },
        ),
        &polygon_query(),
        &|| false,
    )
    .expect("a slow successful provider response is not rejected by total elapsed time");
    assert!(response.products.is_empty());
    assert!(response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert_eq!(response.stats.request_count, 1);
    assert_eq!(time_server.request_count(), 1);
}

#[test]
fn round_robin_paging_reaches_a_later_part_when_the_first_part_has_many_false_positives() {
    let mut query = polygon_query();
    query.result_limit = Some(1);
    query.aoi_geojson = Some(
        json!({
            "type":"Polygon",
            "coordinates":[
                [[80.0,10.0],[140.0,10.0],[140.0,50.0],[80.0,50.0],[80.0,10.0]],
                [[85.0,15.0],[95.0,15.0],[95.0,25.0],[85.0,25.0],[85.0,15.0]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let server = LoopbackServer::start(vec![
        Reply::page(
            500_000,
            vec![granule_with_footprint(60, 86.0, 16.0, 87.0, 17.0)],
        ),
        Reply::page(
            1,
            vec![granule_with_footprint(61, 100.0, 35.0, 101.0, 36.0)],
        ),
    ]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("round-robin transport reaches the second part");

    assert_eq!(response.products.len(), 1);
    assert_eq!(response.products[0].product_id, "G9000000061-ASF");
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.transport_part_count, 4);
    assert!(!response.stats.remote_exhausted);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.contains("page_num=1")));
    assert_ne!(requests[0], requests[1]);
}

#[test]
fn finite_n_scans_bounded_candidates_until_exact_working_set_is_satisfied() {
    let mut query = polygon_query();
    query.result_limit = Some(200);
    let mut ring = (0..160)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / 160.0;
            json!([111.5 + 2.0 * angle.cos(), 31.5 + 2.0 * angle.sin()])
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    query.aoi_geojson = Some(
        json!({"type": "Polygon", "coordinates": [ring]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let first_page = (0..200)
        .map(|index| {
            if index < 73 {
                granule_with_footprint(index, 111.45, 31.45, 111.55, 31.55)
            } else {
                granule_with_footprint(index, 109.51, 29.51, 109.61, 29.61)
            }
        })
        .collect();
    let second_page = (200..400)
        .map(|index| granule_with_footprint(index, 109.51, 29.51, 109.61, 29.61))
        .collect();
    let third_page = (400..600)
        .map(|index| {
            if index < 527 {
                granule_with_footprint(index, 111.45, 31.45, 111.55, 31.55)
            } else {
                granule_with_footprint(index, 109.51, 29.51, 109.61, 29.61)
            }
        })
        .collect();
    let server = LoopbackServer::start(vec![
        Reply::page(500_000, first_page),
        Reply::page(500_000, second_page),
        Reply::page(500_000, third_page),
    ]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("bounded candidate scanning reaches the finite exact working set");

    assert_eq!(response.products.len(), 200);
    assert_eq!(response.stats.requested_limit, Some(200));
    assert_eq!(response.stats.fetched_record_count, 600);
    assert_eq!(response.stats.exact_matched_count, 200);
    assert_eq!(response.stats.returned_count, 200);
    assert_eq!(response.stats.request_count, 3);
    assert!(!response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
}

#[test]
fn finite_working_set_is_bounded_even_when_remote_total_exceeds_500() {
    let server = LoopbackServer::start(vec![Reply::page(500_000, (0..30).map(granule).collect())]);
    let mut query = polygon_query();
    query.result_limit = Some(30);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("finite working set stops at the requested quantity");
    assert_eq!(response.products.len(), 30);
    assert_eq!(response.stats.matched_count, Some(500_000));
    assert_eq!(response.stats.fetched_record_count, 30);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn collection_rows_outside_the_scansar_profile_do_not_abort_a_valid_working_set() {
    let mut query = polygon_query();
    query.result_limit = Some(1);
    let server = LoopbackServer::start(vec![
        Reply::page(2, vec![granule_with_beam_mode(75, "FBS")]),
        Reply::page(2, vec![granule(76)]),
    ]);

    let client = client(&server, CatalogSearchLimits::default());
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");
    let progress = Mutex::new(Vec::<CatalogSearchProviderProgress>::new());
    let response = client
        .search_alos2_with_progress(&query, profile, &|| false, &|sample| {
            progress.lock().unwrap().push(sample);
        })
        .expect("out-of-profile collection rows are bounded candidates, not fatal mapper input");

    assert_eq!(response.products.len(), 1);
    assert_eq!(response.products[0].product_id, "G9000000076-ASF");
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.mapped_record_count, 1);
    assert_eq!(response.stats.profile_filtered_record_count, 1);
    assert_eq!(response.stats.returned_count, 1);
    let progress = progress.lock().unwrap();
    assert_eq!(
        progress.len(),
        2,
        "one exact progress sample follows every CMR request"
    );
    assert_eq!(progress[0].provider_request_count, 1);
    assert_eq!(progress[0].raw_fetched_count, 1);
    assert_eq!(progress[0].mapped_count, 0);
    assert_eq!(progress[0].exact_matched_count, 0);
    assert_eq!(progress[1].provider_request_count, 2);
    assert_eq!(progress[1].raw_fetched_count, 2);
    assert_eq!(progress[1].mapped_count, 1);
    assert_eq!(progress[1].exact_matched_count, 1);
}

#[test]
fn high_vertex_administrative_aoi_uses_bounded_cmr_transport() {
    let mut query = polygon_query();
    query.result_limit = Some(2);
    let mut ring = (0..720)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / 720.0;
            json!([111.5 + 2.0 * angle.cos(), 31.5 + 2.0 * angle.sin()])
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    query.aoi_geojson = Some(
        json!({"type": "Polygon", "coordinates": [ring]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let server = LoopbackServer::start(vec![Reply::page(
        2,
        vec![
            granule_with_footprint(80, 111.0, 31.0, 111.2, 31.2),
            granule_with_footprint(81, 111.8, 31.8, 112.0, 32.0),
        ],
    )]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("a high-vertex authority uses provider-safe conservative transport");

    assert_eq!(response.products.len(), 2);
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "the first part already satisfied N=2");
    assert!(
        requests[0].len() < 8_000,
        "CMR transport must stay bounded instead of embedding all 720 authority vertices"
    );
    assert_eq!(
        response.products[0].properties["spatial_transport_mode"],
        "cmr_conservative_bbox_exact_post_filter"
    );
}

#[test]
fn zero_hit_catalogue_response_is_a_successful_empty_working_set() {
    let server = LoopbackServer::start(vec![Reply::page(0, vec![])]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("zero remote hits are a successful empty search");

    assert!(response.products.is_empty());
    assert_eq!(response.stats.matched_count, Some(0));
    assert_eq!(response.stats.fetched_record_count, 0);
    assert_eq!(response.stats.returned_count, 0);
    assert!(response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn cmr_http_failure_preserves_a_typed_catalogue_code_and_safe_status() {
    let server = LoopbackServer::start(vec![Reply::page(0, vec![]).with_status(400)]);

    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .unwrap_err();

    assert_eq!(error.code, "CATALOGUE_HTTP_CLIENT_FAILURE");
    assert_eq!(error.debug_context["status"], 400);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn cancellation_after_fetch_is_not_published_as_a_partial_batch() {
    let server = LoopbackServer::start(vec![Reply::page(1, vec![granule(60)])]);
    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| server.request_count() > 0,
    )
    .unwrap_err();

    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert_eq!(server.request_count(), 1);
}

#[test]
fn raw_umm_data_url_must_match_the_authoritative_archive_name() {
    let mut invalid = granule(70);
    invalid["umm"]["RelatedUrls"][0]["URL"] = json!("https://example.invalid/alos2/different.zip");
    invalid["umm"]["RelatedUrls"][0]["Description"] = json!("unrelated data URL");
    let server = LoopbackServer::start(vec![Reply::page(1, vec![invalid])]);

    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .unwrap_err();

    assert_eq!(error.code, "ASF_ASSET_CLASSIFICATION_INVALID");
    assert_eq!(
        error.debug_context["reason"],
        "related_url_archive_identity"
    );
}

#[test]
#[ignore = "public ALOS-2 CMR network smoke test"]
fn public_alos2_typed_catalogue_search_maps_a_real_cmr_product() {
    let client = AsfCatalogClient::production().expect("production ASF catalogue client");
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": ALOS2_LOGICAL_DATASET,
        "provider_id": ALOS2_PROVIDER_ID,
        "native_collection_id": ALOS2_NATIVE_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[174.5, 66.5], [176.0, 66.5], [176.0, 68.0], [174.5, 68.0], [174.5, 66.5]]]
        },
        "cursor": null,
        "result_limit": 5,
        "filters": {
            "datetime": {"start":"2014-08-04T00:00:00Z", "end":"2014-08-05T00:00:00Z"},
            "beam_mode": "WBD",
            "polarizations": ["HH", "HV"],
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT"
        }
    }))
    .expect("closed public ALOS-2 query");
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");

    let response = client
        .search_alos2(&query, profile, &|| false)
        .expect("real ALOS-2 typed catalogue search");
    let product = response
        .products
        .iter()
        .find(|product| product.display_name == "ALOS2010632250-140804-WBDR1.1__D")
        .expect("the public CMR sample is mapped");

    assert_eq!(product.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(product.properties["logical_dataset"], ALOS2_LOGICAL_DATASET);
    assert_eq!(
        product.properties["collection_concept_id"],
        ALOS2_NATIVE_COLLECTION_ID
    );
    assert_eq!(product.properties["processing_level"], "1.1");
    assert_eq!(product.properties["beam_mode"], "WBD");
    assert!(!product.assets.is_empty());
}
