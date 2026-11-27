use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use insar_core::CatalogSearchQuery;
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::asf::catalog_client::NISAR_CATALOGUE_CANARY_GRANULE_ID;
use insar_provider::asf::catalog_client::{AsfCatalogClient, CatalogSearchLimits};
use insar_provider::asf::client::{AsfHttpClient, HttpLimits, RemoteEndpoints};
use insar_provider::asf::nisar::build_nisar_rslc_query;
use insar_provider::catalog::{
    CollectionProfileRegistry, DatasetRoutePolicyRegistry, NISAR_LOGICAL_DATASET,
    NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_ROUTE_ID,
};
#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::catalog::{
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct Reply {
    body: Vec<u8>,
    delay: Duration,
}

impl Reply {
    fn page(hits: u64, items: Vec<Value>) -> Self {
        Self {
            body: serde_json::to_vec(&json!({"hits": hits, "items": items})).unwrap(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
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
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
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

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1/granule-provisional-multi-asset-valid.json");
    let mut fixture: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("NISAR fixture exists"))
            .expect("NISAR fixture is valid JSON");
    fixture["collection"]["cmr_provider_id"] = json!("ASF");
    fixture
}

fn granule(index: usize, look_direction: &str) -> Value {
    let mut value = fixture();
    let concept_id = format!("G8{index:09}-ASF");
    let producer = format!("NISAR_L1_PR_RSLC_CATALOG_{index:06}");
    let file_name = format!("{producer}.h5");
    value["granule"]["concept_id"] = json!(concept_id);
    value["granule"]["producer_granule_id"] = json!(producer);
    value["granule"]["asf_product_id"] = value["granule"]["producer_granule_id"].clone();
    value["granule"]["look_direction"] = json!(look_direction);
    value["granule"]["acquisition_start_utc"] = json!("2026-06-18T10:00:00Z");
    value["granule"]["acquisition_end_utc"] = json!("2026-06-18T10:01:30Z");
    let primary = value["granule"]["assets"]["records"]
        .as_array_mut()
        .expect("asset records")
        .iter_mut()
        .find(|asset| asset["normalized_role"] == "primary")
        .expect("primary asset");
    primary["file_name"] = json!(file_name);
    primary["native_evidence"]["archive_name"] = primary["file_name"].clone();
    primary["routes"] = json!([
        {
            "scheme": "https",
            "locator": format!("https://example.invalid/nisar/{file_name}")
        },
        {
            "scheme": "s3",
            "locator": format!("s3://example-invalid/nisar/{file_name}")
        }
    ]);
    value
}

fn granule_with_footprint(index: usize, look_direction: &str, footprint: &Value) -> Value {
    let mut value = granule(index, look_direction);
    value["granule"]["footprint"] = footprint.clone();
    value
}

fn multipart_catalogue_aoi() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[100.0, 10.0], [101.0, 10.0], [101.0, 11.0], [100.0, 11.0], [100.0, 10.0]]],
            [[[140.0, 20.0], [141.0, 20.0], [141.0, 21.0], [140.0, 21.0], [140.0, 20.0]]]
        ]
    })
}

fn first_multipart_component_footprint() -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [100.2, 10.2],
            [100.8, 10.2],
            [100.8, 10.8],
            [100.2, 10.8],
            [100.2, 10.2]
        ]]
    })
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn exact_canary_granule() -> Value {
    let mut value = fixture();
    value["granule"]["concept_id"] = json!("G1234567890-ASF");
    value["granule"]["producer_granule_id"] = json!(NISAR_CATALOGUE_CANARY_GRANULE_ID);
    value["granule"]["asf_product_id"] = json!(NISAR_CATALOGUE_CANARY_GRANULE_ID);
    value["granule"]["footprint"] = fixed_p1_5_aoi();
    let primary = value["granule"]["assets"]["records"]
        .as_array_mut()
        .expect("asset records")
        .iter_mut()
        .find(|asset| asset["normalized_role"] == "primary")
        .expect("primary asset");
    primary["file_name"] = json!(format!("{NISAR_CATALOGUE_CANARY_GRANULE_ID}.h5"));
    primary["native_evidence"]["archive_name"] = primary["file_name"].clone();
    primary["size_bytes"] = json!(14_252_244_992_u64);
    primary["checksum"] = json!({
        "algorithm": "MD5",
        "value": "059ada6521bff1ef314fb1141a89cd10"
    });
    primary["routes"] = json!([
        {
            "scheme": "https",
            "locator": format!("https://example.invalid/nisar/{NISAR_CATALOGUE_CANARY_GRANULE_ID}.h5")
        },
        {
            "scheme": "s3",
            "locator": format!("s3://example-invalid/nisar/{NISAR_CATALOGUE_CANARY_GRANULE_ID}.h5")
        }
    ]);
    value
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn official_single_scene_evidence_binds_the_canary_identity_and_local_aoi() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../docs/fixtures/asf-nisar-subset-p0/official-single-scene-result.json"
    ))
    .expect("official single-scene evidence is valid JSON");
    assert_eq!(
        evidence["source"]["granule_id"],
        NISAR_CATALOGUE_CANARY_GRANULE_ID
    );
    assert_eq!(evidence["source"]["size_bytes"], 14_252_244_992_u64);
    assert_eq!(
        evidence["source"]["sha256"],
        "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0"
    );
    assert_eq!(evidence["source"]["product_specification_version"], "1.5.0");
    assert_eq!(evidence["source"]["composite_release_id"], "P05023");
    assert_eq!(
        evidence["science"]["qa_masks"],
        json!([
            "frequencyA/inputDataExceptionMask",
            "frequencyB/inputDataExceptionMask"
        ])
    );
    let official_aoi = evidence["selection"]["requested_aoi"]
        .as_object()
        .expect("official AOI object");
    let fixed_aoi = fixed_p1_5_aoi();
    let fixed_aoi = fixed_aoi.as_object().expect("fixed AOI object");
    let official_points = official_aoi["coordinates"][0]
        .as_array()
        .expect("official AOI ring");
    let fixed_points = fixed_aoi["coordinates"][0]
        .as_array()
        .expect("fixed AOI ring");
    assert_eq!(official_points.len(), fixed_points.len());
    for (official, fixed) in official_points.iter().zip(fixed_points) {
        let official = official.as_array().expect("official AOI position");
        let fixed = fixed.as_array().expect("fixed AOI position");
        assert_eq!(official.len(), 2);
        assert_eq!(fixed.len(), 2);
        for (official, fixed) in official.iter().zip(fixed) {
            assert!(
                (official.as_f64().expect("official coordinate")
                    - fixed.as_f64().expect("fixed coordinate"))
                .abs()
                    < 1e-12
            );
        }
    }
}

fn polygon_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [113.0, 30.0], [113.0, 33.0], [110.0, 33.0], [110.0, 30.0]]]
        },
        "cursor": null,
        "result_limit": 2,
        "filters": {
            "datetime": {"start":"2026-06-18T00:00:00Z", "end":"2026-06-19T00:00:00Z"},
            "production_configuration": "PR",
            "frame_coverage": "PARTIAL",
            "frequency_a_polarization": "HH+HV",
            "frequency_b_polarization": "HH",
            "range_bandwidth": "20+5",
            "joint_observation": true,
            "flight_direction": "ASCENDING",
            "look_direction": "LEFT",
            "track": 37,
            "frame": 112
        }
    }))
    .expect("closed NISAR catalogue query")
}

fn fixed_p1_5_aoi() -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.85690570018474],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })
}

fn fixed_p1_5_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": fixed_p1_5_aoi(),
        "cursor": null,
        "result_limit": 100,
        "filters": {
            "datetime": {
                "start": "2026-07-26T15:00:00Z",
                "end": "2026-07-26T17:00:00Z"
            },
            "production_configuration": "PR"
        }
    }))
    .expect("fixed P1.5 query is valid")
}

fn polygon_points(value: &str) -> Vec<(f64, f64)> {
    let values = value
        .split(',')
        .map(|value| value.parse::<f64>().expect("CMR polygon number"))
        .collect::<Vec<_>>();
    assert_eq!(values.len() % 2, 0);
    values
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

fn signed_area(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].0 * pair[1].1 - pair[1].0 * pair[0].1)
        .sum::<f64>()
        / 2.0
}

fn query_plan_snapshot() -> Value {
    let query = fixed_p1_5_query();
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let plan = build_nisar_rslc_query(&query, profile).expect("fixed query plan");
    let params = plan.page_params(&plan.polygons[0], 1, plan.provider_page_size);
    let parameter_pairs = params
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect::<Vec<_>>();
    let canonical = serde_json::to_vec(&parameter_pairs).expect("canonical parameters");
    let mut digest = Sha256::new();
    digest.update(canonical);
    json!({
        "schema": "nisar-p1-5-cmr-attempt-2-query-plan-redacted-v1",
        "endpoint_class": "cmr_granules_umm_json",
        "http_method": "GET",
        "collection_concept_id": NISAR_NATIVE_COLLECTION_ID,
        "temporal": "2026-07-26T15:00:00Z,2026-07-26T17:00:00Z",
        "attributes": [
            "string,PRODUCT_TYPE,RSLC",
            "string,PRODUCTION_PIPELINE,PR"
        ],
        "options": {
            "attribute": {
                "exclude_collection": true
            }
        },
        "page_size": 100,
        "page_num": 1,
        "sort_keys": ["start_date", "granule_ur"],
        "polygon": plan.polygons[0],
        "parameter_count": params.len(),
        "parameters": parameter_pairs,
        "canonical_parameter_digest": format!("{:x}", digest.finalize())
    })
}

fn decode_query(request: &str) -> Vec<(String, String)> {
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("HTTP request target");
    let query = target.split_once('?').expect("query string").1;
    url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect()
}

fn fixed_snapshot_parameters() -> Vec<(String, String)> {
    query_plan_snapshot()["parameters"]
        .as_array()
        .expect("snapshot parameters")
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().expect("parameter name").to_owned(),
                pair[1].as_str().expect("parameter value").to_owned(),
            )
        })
        .collect()
}

fn client(server: &LoopbackServer, limits: CatalogSearchLimits) -> AsfCatalogClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    let http = AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default())
        .expect("loopback HTTP client");
    AsfCatalogClient::with_http_client_for_tests(http, limits).expect("bounded catalogue client")
}

fn http_client(server: &LoopbackServer) -> AsfHttpClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default())
        .expect("loopback HTTP client")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn controlled_client(server: &LoopbackServer, limits: CatalogSearchLimits) -> AsfCatalogClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    let http = AsfHttpClient::with_test_endpoints(endpoints, HttpLimits::default())
        .expect("loopback HTTP client");
    AsfCatalogClient::with_http_client_for_nisar_controlled_live_acceptance_for_tests(http, limits)
        .expect("controlled bounded catalogue client")
}

#[cfg(feature = "nisar-subset-live-acceptance")]
fn exact_canary_client(server: &LoopbackServer) -> AsfCatalogClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    let limits = HttpLimits {
        max_json_bytes: 2 * 1024 * 1024,
        ..HttpLimits::default()
    };
    let http = AsfHttpClient::with_test_endpoints(endpoints, limits).expect("loopback HTTP client");
    AsfCatalogClient::with_http_client_for_nisar_exact_identity_canary_for_tests(http)
        .expect("exact identity canary client")
}

fn search(
    client: &AsfCatalogClient,
    query: &CatalogSearchQuery,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<insar_core::CatalogSearchResponse, insar_core::ContractError> {
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    client.search_nisar(query, profile, cancelled)
}

#[test]
fn production_catalogue_route_is_enabled_before_network_access() {
    let route = DatasetRoutePolicyRegistry
        .for_logical_dataset(NISAR_LOGICAL_DATASET)
        .expect("NISAR route is registered");
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR collection profile is registered");

    assert_eq!(route.route_id, NISAR_ROUTE_ID);
    assert!(route.production_enabled);
    assert!(profile.production_enabled);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn controlled_nisar_candidate_reaches_loopback_with_production_enabled() {
    let server = LoopbackServer::start(vec![Reply::page(1, vec![granule(70, "LEFT")])]);
    let response = search(
        &controlled_client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("controlled NISAR candidate reaches loopback catalogue");

    assert_eq!(response.products.len(), 1);
    assert!(response.collection.production_enabled);
    assert_eq!(server.request_count(), 1);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn exact_identity_canary_uses_only_the_three_fixed_query_pairs() {
    let server = LoopbackServer::start(vec![Reply::page(1, vec![exact_canary_granule()])]);
    let result = exact_canary_client(&server)
        .search_nisar_exact_identity_canary(
            NISAR_NATIVE_COLLECTION_ID,
            NISAR_CATALOGUE_CANARY_GRANULE_ID,
            fixed_p1_5_aoi().as_object().expect("fixed AOI object"),
            &|| false,
        )
        .expect("fixed exact identity loopback result");

    assert_eq!(result.request_count, 1);
    assert_eq!(result.http_status, 200);
    assert!(result.response_body_bytes > 0);
    assert!(result.response_body_bytes <= 2 * 1024 * 1024);
    assert_eq!(result.hits, 1);
    assert_eq!(result.returned_item_count, 1);
    assert_eq!(
        result.producer_granule_id,
        NISAR_CATALOGUE_CANARY_GRANULE_ID
    );
    assert_eq!(result.source_size_bytes, 14_252_244_992);
    assert_eq!(result.source_checksum_algorithm, "MD5");
    assert_eq!(
        result.source_checksum_value,
        "059ada6521bff1ef314fb1141a89cd10"
    );
    assert!(result.footprint_valid);
    assert!(result.aoi_intersects);
    assert!(result.primary_locator_present);

    assert_eq!(
        decode_query(&server.requests()[0]),
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned()
            ),
            (
                "producer_granule_id[]".to_owned(),
                NISAR_CATALOGUE_CANARY_GRANULE_ID.to_owned()
            ),
            ("page_size".to_owned(), "2".to_owned()),
        ]
    );
    assert_eq!(server.request_count(), 1);
    for forbidden in [
        "polygon=",
        "temporal=",
        "options[",
        "attribute=",
        "sort_key",
        "page_num=",
        "provider=",
        "short_name=",
        "version=",
        "granule_ur=",
    ] {
        assert!(
            !server.requests()[0].contains(forbidden),
            "unexpected {forbidden}"
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn exact_identity_canary_rejects_wrong_fixed_scope_before_http() {
    let server = LoopbackServer::start(Vec::new());
    let client = exact_canary_client(&server);
    let aoi = fixed_p1_5_aoi();
    let aoi = aoi.as_object().expect("fixed AOI object");
    assert_eq!(
        client
            .search_nisar_exact_identity_canary(
                "wrong-collection",
                NISAR_CATALOGUE_CANARY_GRANULE_ID,
                aoi,
                &|| false,
            )
            .unwrap_err()
            .debug_context["reason"],
        "collection_identity"
    );
    assert_eq!(
        client
            .search_nisar_exact_identity_canary(
                NISAR_NATIVE_COLLECTION_ID,
                "wrong-producer-granule",
                aoi,
                &|| false,
            )
            .unwrap_err()
            .debug_context["reason"],
        "producer_granule_identity"
    );
    assert_eq!(server.request_count(), 0);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn exact_identity_canary_rejects_zero_or_two_hits_without_follow_up_requests() {
    for (hits, items) in [
        (0, Vec::new()),
        (2, vec![exact_canary_granule(), exact_canary_granule()]),
    ] {
        let server = LoopbackServer::start(vec![Reply::page(hits, items)]);
        let aoi = fixed_p1_5_aoi();
        let error = exact_canary_client(&server)
            .search_nisar_exact_identity_canary(
                NISAR_NATIVE_COLLECTION_ID,
                NISAR_CATALOGUE_CANARY_GRANULE_ID,
                aoi.as_object().expect("fixed AOI object"),
                &|| false,
            )
            .expect_err("cardinality must fail closed");
        assert_eq!(error.debug_context["reason"], "cardinality");
        assert_eq!(server.request_count(), 1);
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn exact_identity_canary_rejects_mapped_identity_integrity_and_geometry_mismatches() {
    let mut cases = Vec::new();

    let mut wrong_producer = exact_canary_granule();
    wrong_producer["granule"]["producer_granule_id"] = json!("NISAR_WRONG_PRODUCER");
    cases.push(wrong_producer);

    let mut wrong_collection = exact_canary_granule();
    wrong_collection["collection"]["concept_id"] = json!("C0000000000-ASF");
    cases.push(wrong_collection);

    let mut beta = exact_canary_granule();
    beta["collection"]["concept_id"] = json!("C2850225585-ASF");
    beta["collection"]["short_name"] = json!("NISAR_L1_RSLC_BETA_V1");
    cases.push(beta);

    let mut wrong_crid = exact_canary_granule();
    wrong_crid["granule"]["crid"] = json!("P00000");
    cases.push(wrong_crid);

    let mut wrong_size = exact_canary_granule();
    wrong_size["granule"]["assets"]["records"][0]["size_bytes"] = json!(1_u64);
    cases.push(wrong_size);

    let mut wrong_checksum = exact_canary_granule();
    wrong_checksum["granule"]["assets"]["records"][0]["checksum"]["value"] =
        json!("0000000000000000000000000000000000000000000000000000000000000000");
    cases.push(wrong_checksum);

    let mut invalid_footprint = exact_canary_granule();
    invalid_footprint["granule"]["footprint"] = json!({
        "type": "Point",
        "coordinates": [37.72, 10.85]
    });
    cases.push(invalid_footprint);

    let mut nonoverlap = exact_canary_granule();
    nonoverlap["granule"]["footprint"] = json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 31.0], [110.0, 30.0]]]
    });
    cases.push(nonoverlap);

    for item in cases {
        let server = LoopbackServer::start(vec![Reply::page(1, vec![item])]);
        let aoi = fixed_p1_5_aoi();
        let result = exact_canary_client(&server).search_nisar_exact_identity_canary(
            NISAR_NATIVE_COLLECTION_ID,
            NISAR_CATALOGUE_CANARY_GRANULE_ID,
            aoi.as_object().expect("fixed AOI object"),
            &|| false,
        );
        assert!(result.is_err(), "mismatched canary item was accepted");
        assert_eq!(server.request_count(), 1);
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn exact_identity_canary_enforces_the_two_mib_success_body_limit() {
    let server = LoopbackServer::start(vec![Reply {
        body: vec![b' '; 2 * 1024 * 1024 + 1],
        delay: Duration::ZERO,
    }]);
    let aoi = fixed_p1_5_aoi();
    let error = exact_canary_client(&server)
        .search_nisar_exact_identity_canary(
            NISAR_NATIVE_COLLECTION_ID,
            NISAR_CATALOGUE_CANARY_GRANULE_ID,
            aoi.as_object().expect("fixed AOI object"),
            &|| false,
        )
        .expect_err("oversized catalogue response must fail closed");
    assert_eq!(error.debug_context["category"], "response_too_large");
    assert_eq!(server.request_count(), 1);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn controlled_candidate_rejects_alos2_before_loopback_http() {
    let server = LoopbackServer::start(Vec::new());
    let client = controlled_client(&server, CatalogSearchLimits::default());
    let mut query = polygon_query();
    query.logical_dataset = ALOS2_LOGICAL_DATASET.to_owned();
    query.provider_id = ALOS2_PROVIDER_ID.to_owned();
    query.native_collection_id = ALOS2_NATIVE_COLLECTION_ID.to_owned();
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");

    let error = client
        .search_alos2(&query, profile, &|| false)
        .expect_err("controlled NISAR admission must reject ALOS-2");
    assert_eq!(error.code, "CATALOGUE_ROUTE_DISABLED");
    assert_eq!(server.request_count(), 0);
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn controlled_constructor_is_feature_gated_and_normal_production_stays_standard() {
    let source = include_str!("../src/asf/catalog_client.rs");
    assert!(source.contains("pub fn production() -> ProviderResult<Self> {"));
    assert!(source.contains("CatalogRouteAdmission::Standard(RouteUse::Production)"));
    assert!(source.contains("#[cfg(feature = \"nisar-subset-live-acceptance\")]"));
    assert!(source.contains("pub fn nisar_controlled_live_acceptance()"));
}

#[test]
fn fixed_p1_5_query_snapshot_is_builder_bound_and_counter_clockwise() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../docs/fixtures/asf-nisar-subset-p1-5/cmr-attempt-2-query-plan-redacted.json"
    ))
    .expect("query snapshot is valid JSON");
    let actual = query_plan_snapshot();
    assert_eq!(actual, expected);

    let polygon = polygon_points(actual["polygon"].as_str().expect("polygon snapshot"));
    assert_eq!(polygon.first(), polygon.last());
    assert!(signed_area(&polygon) > 0.0);
}

#[test]
fn loopback_query_matrix_preserves_pairs_duplicates_and_canonical_polygon() {
    let polygon = fixed_snapshot_parameters()
        .into_iter()
        .find(|(name, _)| name == "polygon")
        .expect("fixed polygon")
        .1;
    let cases = vec![
        vec![(
            "collection_concept_id".to_owned(),
            NISAR_NATIVE_COLLECTION_ID.to_owned(),
        )],
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned(),
            ),
            (
                "temporal".to_owned(),
                "2026-07-26T15:00:00Z,2026-07-26T17:00:00Z".to_owned(),
            ),
        ],
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned(),
            ),
            ("polygon".to_owned(), polygon.clone()),
        ],
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned(),
            ),
            (
                "attribute[]".to_owned(),
                "string,PRODUCT_TYPE,RSLC".to_owned(),
            ),
        ],
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned(),
            ),
            (
                "attribute[]".to_owned(),
                "string,PRODUCTION_PIPELINE,PR".to_owned(),
            ),
        ],
        vec![
            (
                "collection_concept_id".to_owned(),
                NISAR_NATIVE_COLLECTION_ID.to_owned(),
            ),
            (
                "attribute[]".to_owned(),
                "string,PRODUCT_TYPE,RSLC".to_owned(),
            ),
            (
                "attribute[]".to_owned(),
                "string,PRODUCTION_PIPELINE,PR".to_owned(),
            ),
        ],
        fixed_snapshot_parameters(),
    ];
    let replies = (0..cases.len()).map(|_| Reply::page(0, vec![])).collect();
    let server = LoopbackServer::start(replies);
    let client = http_client(&server);
    for case in &cases {
        client
            .cmr_json(case, &|| false)
            .expect("loopback matrix response");
    }

    let requests = server.requests();
    assert_eq!(requests.len(), cases.len());
    for (request, expected) in requests.iter().zip(cases.iter()) {
        assert_eq!(decode_query(request), *expected);
    }
    let full = &cases[cases.len() - 1];
    assert_eq!(
        full.iter()
            .filter(|(name, _)| name == "attribute[]")
            .count(),
        2
    );
    assert_eq!(
        full.iter().filter(|(name, _)| name == "sort_key[]").count(),
        2
    );
    assert_eq!(
        full.iter().find(|(name, _)| name == "page_num").unwrap().1,
        "1"
    );
    assert_eq!(
        full.iter().find(|(name, _)| name == "page_size").unwrap().1,
        "100"
    );
    assert!(signed_area(&polygon_points(&polygon)) > 0.0);
}

#[test]
fn duplicate_terminal_vertex_is_rejected_by_authoritative_geometry() {
    let mut query = fixed_p1_5_query();
    query.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[
                [0.0, 0.0],
                [1.0, 0.0],
                [1.0, 1.0],
                [0.0, 1.0],
                [0.0, 0.0],
                [0.0, 0.0]
            ]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let error = build_nisar_rslc_query(&query, profile)
        .expect_err("duplicate terminal vertex must fail authoritative validation");
    assert_eq!(error.code, "CATALOGUE_FILTER_INVALID");
    assert_eq!(error.debug_context["field"], "aoi_geojson");
    assert_eq!(
        error.debug_context["reason"],
        "authoritative_polygon_invalid"
    );
}

#[test]
fn degenerate_polygon_is_rejected_without_bbox_fallback() {
    let mut query = fixed_p1_5_query();
    query.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [0.0, 0.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let error = build_nisar_rslc_query(&query, profile).expect_err("zero-area polygon fails");
    assert_eq!(error.code, "CATALOGUE_FILTER_INVALID");
    assert_eq!(
        error.debug_context["reason"],
        "authoritative_polygon_invalid"
    );
    assert!(!error.debug_context.contains_key("bbox"));
}

#[test]
fn multipolygon_preserves_canonical_components_with_conservative_transport() {
    let mut query = fixed_p1_5_query();
    query.aoi_geojson = Some(
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]],
                [[[2.0, 0.0], [3.0, 0.0], [3.0, 1.0], [2.0, 1.0], [2.0, 0.0]]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let plan = build_nisar_rslc_query(&query, profile).expect("canonical multipolygon");
    assert_eq!(plan.canonical_polygon_count, 2);
    assert!(plan.requires_exact_spatial_post_filter);
    assert_eq!(plan.transport_part_count, 1);
    assert_eq!(
        plan.spatial_transport_mode,
        "cmr_conservative_bbox_exact_post_filter"
    );
    assert_eq!(plan.polygons.len(), plan.transport_part_count);
    assert_eq!(
        plan.canonical_geometry
            .as_ref()
            .expect("canonical geometry")
            .0
            .len(),
        2
    );
    let transport_points = polygon_points(&plan.polygons[0]);
    assert_eq!(transport_points.len(), 5);
    assert_eq!(transport_points.first(), transport_points.last());
    assert!(signed_area(&transport_points) > 0.0);
}

#[test]
fn two_pages_complete_before_local_look_filter_and_preserve_every_count() {
    let server = LoopbackServer::start(vec![
        Reply::page(4, vec![granule(1, "LEFT"), granule(2, "RIGHT")]),
        Reply::page(4, vec![granule(3, "LEFT"), granule(4, "RIGHT")]),
    ]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("complete two-page NISAR search");

    assert_eq!(
        response.collection.native_collection_id,
        NISAR_NATIVE_COLLECTION_ID
    );
    assert_eq!(response.stats.matched_count, Some(4));
    assert_eq!(response.stats.fetched_record_count, 4);
    assert_eq!(response.stats.mapped_record_count, 4);
    assert_eq!(response.stats.exact_duplicate_record_count, 0);
    assert_eq!(response.stats.unique_provider_product_count, 4);
    assert_eq!(response.stats.returned_count, 2);
    assert!(!response.stats.truncated);
    assert!(response.next_cursor.is_none());
    assert!(response.item_errors.is_empty());
    assert!(response.products.iter().all(|product| {
        product.properties["look_direction"] == "LEFT"
            && product.properties["quality_status"] == "Provisional"
    }));

    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        assert!(request.contains("collection_concept_id=C2853145197-ASF"));
        assert!(request.contains("page_size=2"));
        assert!(request.contains(&format!("page_num={}", index + 1)));
        assert!(request.contains("attribute%5B%5D=string%2CPRODUCT_TYPE%2CRSLC"));
        assert!(request.contains("attribute%5B%5D=string%2CPRODUCTION_PIPELINE%2CPR"));
        assert!(!request.contains("LOOK_DIRECTION"));
        assert!(request.contains("polygon="));
    }
}

#[test]
fn scope_mismatch_is_published_as_item_error_and_does_not_abort_batch() {
    let mut out_of_scope = granule(2, "LEFT");
    out_of_scope["granule"]["band"] = json!("S");
    let server =
        LoopbackServer::start(vec![Reply::page(2, vec![granule(1, "LEFT"), out_of_scope])]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("scope mismatch remains item-local");

    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.mapped_record_count, 1);
    assert_eq!(response.stats.profile_filtered_record_count, 0);
    assert_eq!(response.stats.unique_provider_product_count, 1);
    assert_eq!(response.stats.returned_count, 1);
    assert_eq!(response.item_errors.len(), 1);
    assert_eq!(response.item_errors[0].code, "ASF_PRODUCT_SCOPE_MISMATCH");
    assert_eq!(
        response.item_errors[0].debug_context["reason"],
        "product_scope"
    );
}

#[test]
fn finite_working_set_stops_before_unneeded_overlapping_pages() {
    let duplicate = granule(2, "LEFT");
    let server = LoopbackServer::start(vec![
        Reply::page(4, vec![granule(1, "LEFT"), duplicate.clone()]),
        Reply::page(4, vec![duplicate, granule(3, "LEFT")]),
    ]);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .expect("requested working set is complete after the first page");
    assert_eq!(response.products.len(), 2);
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn multipolygon_merges_by_granule_identity_without_summing_overlapping_hits() {
    let mut query = polygon_query();
    query.result_limit = Some(5);
    query.aoi_geojson = Some(
        multipart_catalogue_aoi()
            .as_object()
            .expect("MultiPolygon AOI")
            .clone(),
    );
    let duplicate = granule_with_footprint(10, "LEFT", &first_multipart_component_footprint());
    let server = LoopbackServer::start(vec![
        Reply::page(1, vec![duplicate.clone()]),
        Reply::page(1, vec![duplicate]),
    ]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("complete MultiPolygon search");

    assert_eq!(response.stats.matched_count, None);
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.exact_duplicate_record_count, 1);
    assert_eq!(response.stats.unique_provider_product_count, 1);
    assert_eq!(response.stats.returned_count, 1);
    assert!(response.stats.transport_part_count > 1);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.contains("polygon=")));
    let request_polygons = requests
        .iter()
        .map(|request| {
            decode_query(request)
                .into_iter()
                .find(|(name, _)| name == "polygon")
                .expect("transport polygon")
                .1
        })
        .collect::<Vec<_>>();
    assert_ne!(request_polygons[0], request_polygons[1]);
}

#[test]
fn cursor_and_repeated_pages_fail_without_partial_publication() {
    let idle = LoopbackServer::start(vec![Reply::page(0, vec![])]);
    let mut continuation = polygon_query();
    continuation.cursor = Some("caller-page-2".to_owned());
    assert_eq!(
        search(
            &client(&idle, CatalogSearchLimits::default()),
            &continuation,
            &|| false
        )
        .unwrap_err()
        .code,
        "CATALOGUE_CURSOR_UNSUPPORTED"
    );
    assert_eq!(idle.request_count(), 0);
    drop(idle);

    let repeated = granule(20, "RIGHT");
    let mut changed = repeated.clone();
    changed["granule"]["pge_version"] = json!("R99.99.9");
    let server = LoopbackServer::start(vec![
        Reply::page(2, vec![repeated]),
        Reply::page(2, vec![changed]),
    ]);
    let mut query = polygon_query();
    query.result_limit = Some(1);
    assert_eq!(
        search(
            &client(&server, CatalogSearchLimits::default()),
            &query,
            &|| false
        )
        .unwrap_err()
        .code,
        "CATALOGUE_PAGINATION_INVALID"
    );
    assert_eq!(server.request_count(), 2);
}

#[test]
fn request_page_record_and_time_budgets_fail_closed() {
    let inconsistent_server = LoopbackServer::start(vec![Reply::page(
        1,
        vec![granule(29, "LEFT"), granule(30, "LEFT")],
    )]);
    let error = search(
        &client(&inconsistent_server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| false,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_PAGINATION_INVALID");

    let page_server = LoopbackServer::start(vec![Reply::page(
        3,
        vec![granule(30, "LEFT"), granule(31, "RIGHT")],
    )]);
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

    let mut multipolygon = polygon_query();
    multipolygon.aoi_geojson = Some(
        multipart_catalogue_aoi()
            .as_object()
            .expect("MultiPolygon AOI")
            .clone(),
    );
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    let plan = build_nisar_rslc_query(&multipolygon, profile).expect("multi-part request plan");
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

    let record_server = LoopbackServer::start(vec![Reply::page(2, vec![granule(40, "LEFT")])]);
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
}

#[test]
fn progressing_pages_are_not_discarded_by_a_total_elapsed_budget() {
    let server = LoopbackServer::start(vec![
        Reply::page(2, vec![granule(50, "RIGHT")]).delayed(Duration::from_millis(20)),
        Reply::page(2, vec![granule(51, "LEFT")]).delayed(Duration::from_millis(20)),
    ]);
    let mut query = polygon_query();
    query.result_limit = Some(1);

    let response = search(
        &client(
            &server,
            CatalogSearchLimits {
                max_progress_idle: Duration::from_millis(5),
                ..CatalogSearchLimits::default()
            },
        ),
        &query,
        &|| false,
    )
    .expect("successful provider progress must continue until the finite target is satisfied");

    assert_eq!(response.products.len(), 1);
    assert_eq!(
        response.products[0].display_name,
        "NISAR_L1_PR_RSLC_CATALOG_000051"
    );
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(server.request_count(), 2);
}

#[test]
fn national_aoi_scans_one_full_fair_page_per_transport_part() {
    let mut replies = (0..7)
        .map(|page| {
            Reply::page(
                50,
                (page * 50..(page + 1) * 50)
                    .map(|index| granule(index, "RIGHT"))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    replies.push(Reply::page(
        50,
        (350..400).map(|index| granule(index, "LEFT")).collect(),
    ));
    let server = LoopbackServer::start(replies);
    let mut query = polygon_query();
    query.result_limit = Some(50);
    query.aoi_geojson = Some(
        json!({
            "type":"Polygon",
            "coordinates":[
                [[70.0,0.0],[140.0,0.0],[140.0,55.0],[70.0,55.0],[70.0,0.0]],
                [[80.0,5.0],[90.0,5.0],[90.0,15.0],[80.0,15.0],[80.0,5.0]]
            ]
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("a national AOI may fairly scan each bounded transport part");

    assert_eq!(response.products.len(), 50);
    assert_eq!(response.stats.transport_part_count, 8);
    assert_eq!(response.stats.request_count, 8);
    assert_eq!(response.stats.fetched_record_count, 400);
    assert!(server
        .requests()
        .iter()
        .all(|request| request.contains("page_size=50")));
}

#[test]
fn finite_working_set_is_bounded_even_when_remote_total_exceeds_500() {
    let server = LoopbackServer::start(vec![Reply::page(
        500,
        (0..50).map(|index| granule(index, "LEFT")).collect(),
    )]);
    let mut query = polygon_query();
    query.result_limit = Some(50);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("finite working set stops at the requested quantity");
    assert_eq!(response.products.len(), 50);
    assert_eq!(response.stats.matched_count, Some(500));
    assert_eq!(response.stats.fetched_record_count, 50);
    assert_eq!(response.stats.returned_count, 50);
    assert_eq!(server.request_count(), 1);

    let server = LoopbackServer::start(vec![Reply::page(
        501,
        (0..50).map(|index| granule(index, "LEFT")).collect(),
    )]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("remote total does not drive a finite working-set scan");
    assert_eq!(response.products.len(), 50);
    assert_eq!(response.stats.matched_count, Some(501));
    assert_eq!(response.stats.fetched_record_count, 50);
    assert_eq!(server.request_count(), 1);

    // The final subcase covers multi-part early stop, independently of the
    // remote-total assertions above. Multi-part provider pages are capped at 50.
    let mut multipolygon = polygon_query();
    multipolygon.result_limit = Some(50);
    multipolygon.aoi_geojson = Some(
        multipart_catalogue_aoi()
            .as_object()
            .expect("MultiPolygon AOI")
            .clone(),
    );
    let footprint = first_multipart_component_footprint();
    let server = LoopbackServer::start(vec![
        Reply::page(
            500,
            (0..50)
                .map(|index| granule_with_footprint(index, "LEFT", &footprint))
                .collect(),
        ),
        Reply::page(1, vec![granule_with_footprint(500, "LEFT", &footprint)]),
    ]);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &multipolygon,
        &|| false,
    )
    .expect("first polygon can fill the finite working set");
    assert_eq!(response.stats.requested_limit, Some(50));
    assert_eq!(response.products.len(), 50);
    assert_eq!(response.stats.fetched_record_count, 50);
    assert_eq!(response.stats.exact_matched_count, 50);
    assert_eq!(response.stats.returned_count, 50);
    assert_eq!(response.stats.request_count, 1);
    assert!(response.stats.transport_part_count > 1);
    assert!(response.stats.working_set_satisfied);
    assert_eq!(server.request_count(), 1);
}

#[test]
fn finite_n250_can_scan_bounded_candidates_beyond_the_returned_batch_limit() {
    let server = LoopbackServer::start(vec![
        Reply::page(750, (0..250).map(|index| granule(index, "RIGHT")).collect()),
        Reply::page(
            750,
            (250..500).map(|index| granule(index, "RIGHT")).collect(),
        ),
        Reply::page(
            750,
            (500..750).map(|index| granule(index, "LEFT")).collect(),
        ),
    ]);
    let mut query = polygon_query();
    query.result_limit = Some(250);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("N250 may scan a larger bounded candidate window for local filters");

    assert_eq!(response.products.len(), 250);
    assert_eq!(response.stats.matched_count, Some(750));
    assert_eq!(response.stats.fetched_record_count, 750);
    assert_eq!(response.stats.exact_matched_count, 250);
    assert_eq!(response.stats.returned_count, 250);
    assert_eq!(server.request_count(), 3);
    assert!(server
        .requests()
        .iter()
        .all(|request| request.contains("page_size=250")));
}

#[test]
fn finite_n250_tolerates_live_remote_hit_count_drift_between_pages() {
    let server = LoopbackServer::start(vec![
        Reply::page(750, (0..250).map(|index| granule(index, "RIGHT")).collect()),
        Reply::page(
            751,
            (250..500).map(|index| granule(index, "RIGHT")).collect(),
        ),
        Reply::page(
            751,
            (500..750).map(|index| granule(index, "LEFT")).collect(),
        ),
    ]);
    let mut query = polygon_query();
    query.result_limit = Some(250);

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("finite working sets tolerate normal CMR ingestion between pages");

    assert_eq!(response.products.len(), 250);
    assert_eq!(response.stats.matched_count, Some(751));
    assert_eq!(response.stats.fetched_record_count, 750);
    assert_eq!(server.request_count(), 3);
}

#[test]
fn local_filter_candidate_budget_never_publishes_a_false_zero_hit_batch() {
    let replies = (0..4)
        .map(|page| {
            Reply::page(
                500,
                (0..50)
                    .map(|index| granule(page * 50 + index, "RIGHT"))
                    .collect(),
            )
        })
        .collect();
    let server = LoopbackServer::start(replies);
    let mut query = polygon_query();
    query.result_limit = Some(50);

    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect_err("a budget-stopped local filter cannot claim a complete zero-hit batch");

    assert_eq!(error.code, "CATALOGUE_WORKING_SET_INCOMPLETE");
    assert_eq!(server.request_count(), 4);
}

#[test]
fn finite_result_limits_are_independent_from_the_asf_transport_page_size() {
    for requested in [1_usize, 17, 250, 251, 617, 3_000, 10_000] {
        let provider_page_size = requested.min(250);
        let replies = (0..requested)
            .collect::<Vec<_>>()
            .chunks(provider_page_size)
            .map(|chunk| {
                Reply::page(
                    requested as u64,
                    chunk.iter().map(|index| granule(*index, "LEFT")).collect(),
                )
            })
            .collect();
        let server = LoopbackServer::start(replies);
        let mut query = polygon_query();
        query.result_limit = Some(requested as u64);

        let response = search(
            &client(&server, CatalogSearchLimits::default()),
            &query,
            &|| false,
        )
        .unwrap_or_else(|error| panic!("finite N={requested} failed: {error:?}"));

        assert_eq!(response.stats.requested_limit, Some(requested as u64));
        assert_eq!(response.products.len(), requested);
        assert_eq!(response.stats.returned_count, requested as u64);
        assert_eq!(response.stats.exact_matched_count, requested as u64);
        assert!(response.stats.working_set_satisfied);
        assert_eq!(
            server.request_count(),
            requested.div_ceil(provider_page_size)
        );
        assert!(server
            .requests()
            .iter()
            .all(|request| request.contains(&format!("page_size={provider_page_size}"))));
    }
}

/// #22 B slice 3(a): generated CMR pages of 250 out of `available`.
fn generated_replies(available: usize) -> Vec<Reply> {
    (0..available)
        .collect::<Vec<_>>()
        .chunks(250)
        .map(|chunk| {
            Reply::page(
                available as u64,
                chunk.iter().map(|index| granule(*index, "LEFT")).collect(),
            )
        })
        .collect()
}

#[test]
fn ten_thousand_target_over_ten_thousand_and_one_keeps_n_without_proving_exhaustion() {
    for requested in [10_000_u64, 10_001] {
        let server = LoopbackServer::start(generated_replies(10_001));
        let mut query = polygon_query();
        query.result_limit = Some(requested);
        let response = search(
            &client(&server, CatalogSearchLimits::default()),
            &query,
            &|| false,
        )
        .unwrap();
        assert_eq!(response.stats.requested_limit, Some(10_000));
        assert_eq!(response.products.len(), 10_000);
        assert_eq!(response.stats.matched_count, Some(10_001));
        assert!(!response.stats.remote_exhausted);
        assert_eq!(server.request_count(), 40);
    }
    let server = LoopbackServer::start(generated_replies(10_000));
    let mut query = polygon_query();
    query.result_limit = Some(10_000);
    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .unwrap();
    assert_eq!(response.products.len(), 10_000);
    assert!(response.stats.remote_exhausted);
}

#[test]
fn all_exhausts_more_than_ten_thousand_without_silent_truncation() {
    let expected = 10_001_usize;
    let provider_page_size = 250_usize;
    let replies = (0..expected)
        .collect::<Vec<_>>()
        .chunks(provider_page_size)
        .map(|chunk| {
            Reply::page(
                expected as u64,
                chunk.iter().map(|index| granule(*index, "LEFT")).collect(),
            )
        })
        .collect();
    let server = LoopbackServer::start(replies);
    let mut query = polygon_query();
    query.result_limit = None;

    let response = search(
        &client(&server, CatalogSearchLimits::default()),
        &query,
        &|| false,
    )
    .expect("ALL must prove remote exhaustion beyond the finite 10,000 ceiling");

    assert_eq!(response.stats.requested_limit, None);
    assert_eq!(response.products.len(), expected);
    assert_eq!(response.stats.returned_count, expected as u64);
    assert_eq!(response.stats.exact_matched_count, expected as u64);
    assert!(response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    assert_eq!(
        server.request_count(),
        expected.div_ceil(provider_page_size)
    );
    assert!(server
        .requests()
        .iter()
        .all(|request| request.contains(&format!("page_size={provider_page_size}"))));
}

#[test]
fn all_resource_protection_fails_closed_instead_of_publishing_a_partial_batch() {
    let server = LoopbackServer::start(vec![Reply::page(
        251,
        (0..250).map(|index| granule(index, "LEFT")).collect(),
    )]);
    let mut query = polygon_query();
    query.result_limit = None;

    let error = search(
        &client(
            &server,
            CatalogSearchLimits {
                max_records: 250,
                ..CatalogSearchLimits::default()
            },
        ),
        &query,
        &|| false,
    )
    .expect_err("ALL cannot publish a partial batch after resource protection fires");

    assert_eq!(error.code, "CATALOGUE_RESULT_LIMIT_EXCEEDED");
    assert_eq!(server.request_count(), 1);
}

#[test]
fn cancellation_after_fetch_never_publishes_a_partial_batch() {
    let server = LoopbackServer::start(vec![Reply::page(1, vec![granule(60, "LEFT")])]);
    let error = search(
        &client(&server, CatalogSearchLimits::default()),
        &polygon_query(),
        &|| server.request_count() > 0,
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert_eq!(server.request_count(), 1);
}
