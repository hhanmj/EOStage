use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use insar_provider::asf::client::{AsfHttpClient, HttpLimits, RemoteEndpoints};
use insar_provider::asf::search::MetadataTransport;
use serde_json::Value;

#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    delay: Duration,
    chunk_size: usize,
    chunk_delay: Duration,
}

impl Reply {
    fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            headers: Vec::new(),
            delay: Duration::ZERO,
            chunk_size: usize::MAX,
            chunk_delay: Duration::ZERO,
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

struct LoopbackServer {
    url: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    streaming_started: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackServer {
    fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let address = listener.local_addr().expect("loopback address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let streaming_started = Arc::new(AtomicBool::new(false));
        let worker_streaming_started = Arc::clone(&streaming_started);
        let worker = thread::spawn(move || {
            for reply in replies {
                listener
                    .set_nonblocking(true)
                    .expect("nonblocking listener");
                let (mut stream, _) = loop {
                    if worker_stop.load(Ordering::Acquire) {
                        return;
                    }
                    match listener.accept() {
                        Ok(stream) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("accept test request: {error}"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking test request stream");
                let request = read_request(&mut stream);
                captured.lock().expect("request lock").push(request);
                thread::sleep(reply.delay);
                let reason = if reply.status == 200 { "OK" } else { "Error" };
                let extra_headers = reply
                    .headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect::<String>();
                let header = format!(
                    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: application/json\r\n{}Connection: close\r\n\r\n",
                    reply.status,
                    reason,
                    reply.body.len(),
                    extra_headers,
                );
                if stream.write_all(header.as_bytes()).is_err() {
                    continue;
                }
                for chunk in reply.body.chunks(reply.chunk_size) {
                    if stream.write_all(chunk).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                    worker_streaming_started.store(true, Ordering::Release);
                    thread::sleep(reply.chunk_delay);
                }
            }
        });
        Self {
            url: format!("http://{address}/search"),
            requests,
            stop,
            streaming_started,
            worker: Some(worker),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request lock")
            .iter()
            .map(|request| String::from_utf8_lossy(request).into_owned())
            .collect()
    }

    fn streaming_started(&self) -> bool {
        self.streaming_started.load(Ordering::Acquire)
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
    let mut expected = None;
    loop {
        let count = stream.read(&mut buffer).unwrap_or(0);
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if expected.is_none() {
            if let Some(header_end) = find_bytes(&request, b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .or_else(|| line.strip_prefix("Content-Length:"))
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                expected = Some(header_end + 4 + content_length);
            }
        }
        if expected.is_some_and(|expected| request.len() >= expected) {
            break;
        }
    }
    request
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn client(server: &LoopbackServer, limits: HttpLimits) -> AsfHttpClient {
    let endpoints =
        RemoteEndpoints::loopback_for_tests(&server.url, &server.url).expect("loopback endpoints");
    AsfHttpClient::with_test_endpoints(endpoints, limits).expect("test client")
}

fn params() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("dataset".to_string(), "SENTINEL-1".to_string()),
        (
            "intersectsWith".to_string(),
            "POLYGON ((0 0, 1 0, 0 0))".to_string(),
        ),
        ("output".to_string(), "geojson".to_string()),
    ])
}

#[test]
fn production_endpoints_are_fixed_official_https_urls() {
    let endpoints = RemoteEndpoints::production();
    assert_eq!(
        endpoints.asf_search_url(),
        "https://api.daac.asf.alaska.edu/services/search/param"
    );
    assert_eq!(
        endpoints.cmr_granules_url(),
        "https://cmr.earthdata.nasa.gov/search/granules.json"
    );
    assert_eq!(
        endpoints.cmr_umm_granules_url(),
        "https://cmr.earthdata.nasa.gov/search/granules.umm_json"
    );
    assert!(!format!("{endpoints:?}").contains("example.invalid"));
    assert!(RemoteEndpoints::loopback_for_tests(
        "https://example.invalid/search",
        "http://127.0.0.1/cmr"
    )
    .is_err());
    assert!(RemoteEndpoints::loopback_for_tests(
        "http://user:secret@127.0.0.1/search",
        "http://127.0.0.1/cmr"
    )
    .is_err());
}

#[test]
fn cmr_umm_page_uses_search_after_and_preserves_timeout_metadata() {
    let server = LoopbackServer::start(vec![Reply::json(200, r#"{"hits":42,"items":[]}"#)
        .with_header("CMR-Search-After", r#"["asf",123,456]"#)
        .with_header("CMR-Time-Out", "true")]);
    let client = client(&server, HttpLimits::default());
    let params = vec![
        (
            "collection_concept_id".to_owned(),
            "C2709161906-ASF".to_owned(),
        ),
        ("page_size".to_owned(), "250".to_owned()),
        ("page_num".to_owned(), "9".to_owned()),
    ];

    let page =
        MetadataTransport::cmr_umm_page(&client, &params, Some(r#"["asf",100,200]"#), &|| false)
            .expect("typed CMR page");

    assert_eq!(page.value["hits"], 42);
    assert_eq!(page.search_after.as_deref(), Some(r#"["asf",123,456]"#));
    assert!(page.timed_out);
    assert!(page.search_after_capable);
    let request = server.requests().pop().expect("captured request");
    let request_lower = request.to_ascii_lowercase();
    assert!(request.contains("page_size=250"));
    assert!(!request.contains("page_num"));
    assert!(request_lower.contains("cmr-search-after: [\"asf\",100,200]"));
}

#[test]
fn polygonal_asf_geojson_posts_directly_and_preserves_parameters() {
    let server = LoopbackServer::start(vec![Reply::json(
        200,
        r#"{"type":"FeatureCollection","features":[]}"#,
    )]);
    let client = client(&server, HttpLimits::default());

    let value = client
        .asf_geojson(&params(), &|| false)
        .expect("direct POST succeeds");

    assert_eq!(value["type"], "FeatureCollection");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
    assert!(requests[0].contains("dataset=SENTINEL-1"));
    assert!(requests[0].contains("intersectsWith=POLYGON"));
}

#[test]
fn polygonal_asf_geojson_retries_one_transient_post_once() {
    let server = LoopbackServer::start(vec![
        Reply::json(503, "temporary-post-failure"),
        Reply::json(200, r#"{"type":"FeatureCollection","features":[]}"#),
    ]);
    let client = client(&server, HttpLimits::default());

    let result = client.asf_geojson(&params(), &|| false);
    if result.is_err() {
        let address = server
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        drop(TcpStream::connect(address));
    }
    let value = result.expect("the single bounded retry succeeds");

    assert_eq!(value["type"], "FeatureCollection");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.starts_with("POST /search ")));
}

#[test]
fn polygonal_asf_geojson_400_is_terminal_after_one_post() {
    let body = r#"{"error_code":"INVALID_PARAMETER"}"#;
    let server = LoopbackServer::start((0..6).map(|_| Reply::json(400, body)).collect());
    let client = client(&server, HttpLimits::default());

    let error = client
        .asf_geojson(&params(), &|| false)
        .expect_err("a rejected query fails without repeated rounds");

    assert_eq!(error.debug_context["status"], 400);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
}

#[test]
fn polygonal_asf_geojson_stops_after_one_retryable_post_retry() {
    let server = LoopbackServer::start(
        (0..6)
            .map(|_| Reply::json(503, "temporarily unavailable"))
            .collect(),
    );
    let client = client(&server, HttpLimits::default());

    let error = client
        .asf_geojson(&params(), &|| false)
        .expect_err("the bounded POST budget is exhausted after one retry");

    assert_eq!(error.code, "ASF_SERVER_ERROR");
    assert_eq!(error.debug_context["status"], 503);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.starts_with("POST /search ")));
}

#[test]
fn rate_limit_is_typed_and_retry_after_drives_one_bounded_retry() {
    let server = LoopbackServer::start(vec![
        Reply::json(429, r#"{"error_code":"RATE_LIMITED"}"#).with_header("Retry-After", "1"),
        Reply::json(429, r#"{"error_code":"RATE_LIMITED"}"#).with_header("Retry-After", "1"),
    ]);
    let client = client(&server, HttpLimits::default());
    let started = std::time::Instant::now();

    let error = client
        .asf_geojson(&params(), &|| false)
        .expect_err("bounded retry budget remains exhausted");

    assert_eq!(error.code, "ASF_RATE_LIMITED");
    assert_eq!(error.debug_context["status"], 429);
    assert_eq!(error.debug_context["retry_after_seconds"], 1);
    assert!(started.elapsed() >= Duration::from_millis(900));
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn http_request_timeout_is_typed_and_bounded() {
    let server = LoopbackServer::start(vec![
        Reply::json(408, r#"{"error_code":"REQUEST_TIMEOUT"}"#),
        Reply::json(408, r#"{"error_code":"REQUEST_TIMEOUT"}"#),
    ]);
    let client = client(&server, HttpLimits::default());

    let error = client
        .asf_geojson(&params(), &|| false)
        .expect_err("bounded retry budget remains exhausted");

    assert_eq!(error.code, "ASF_TIMEOUT");
    assert_eq!(error.debug_context["status"], 408);
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn count_accepts_plain_and_structured_v219_responses() {
    let server = LoopbackServer::start(vec![
        Reply::json(200, "640"),
        Reply::json(200, r#"{"totalResults":"27"}"#),
    ]);
    let client = client(&server, HttpLimits::default());

    assert_eq!(
        client.asf_count(&params(), &|| false).expect("plain count"),
        Some(640)
    );
    assert_eq!(
        client
            .asf_count(&params(), &|| false)
            .expect("structured count"),
        Some(27)
    );
    for request in server.requests() {
        assert!(request.starts_with("POST /search "));
        assert!(request.contains("output=count"));
        assert!(!request.contains("maxResults"));
    }
}

#[test]
fn large_aoi_count_uses_form_post_without_attempting_an_oversized_get() {
    let server = LoopbackServer::start(vec![Reply::json(200, "148")]);
    let client = client(&server, HttpLimits::default());
    let mut large = params();
    large.insert("intersectsWith".to_string(), "X".repeat(20_000));

    assert_eq!(client.asf_count(&large, &|| false).unwrap(), Some(148));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
}

#[test]
fn encoded_get_length_switches_to_post_before_wkt_expansion_exceeds_the_limit() {
    let server = LoopbackServer::start(vec![Reply::json(200, "7")]);
    let client = client(&server, HttpLimits::default());
    let mut encoded_large = params();
    encoded_large.insert("intersectsWith".to_string(), "(".repeat(800));

    assert_eq!(
        client.asf_count(&encoded_large, &|| false).unwrap(),
        Some(7)
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
}

#[test]
fn large_aoi_failure_retries_post_once_without_falling_back_to_an_oversized_get() {
    let server = LoopbackServer::start(
        (0..6)
            .map(|_| Reply::json(503, "temporarily unavailable"))
            .collect(),
    );
    let client = client(&server, HttpLimits::default());
    let mut large = params();
    large.insert("intersectsWith".to_string(), "X".repeat(20_000));

    assert_eq!(client.asf_count(&large, &|| false).unwrap(), None);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.starts_with("POST /search ")));
}

#[test]
fn large_aoi_count_does_not_retry_a_rejected_post_query() {
    let server = LoopbackServer::start(
        (0..3)
            .map(|_| Reply::json(400, r#"{"error_code":"INVALID_PARAMETER"}"#))
            .collect(),
    );
    let client = client(&server, HttpLimits::default());
    let mut large = params();
    large.insert("intersectsWith".to_string(), "X".repeat(20_000));

    assert_eq!(client.asf_count(&large, &|| false).unwrap(), None);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
}

#[test]
fn large_aoi_geojson_does_not_retry_a_rejected_post_query() {
    let server = LoopbackServer::start(
        (0..3)
            .map(|_| Reply::json(400, r#"{"error_code":"INVALID_PARAMETER"}"#))
            .collect(),
    );
    let client = client(&server, HttpLimits::default());
    let mut large = params();
    large.insert("intersectsWith".to_string(), "X".repeat(20_000));

    let error = client
        .asf_geojson(&large, &|| false)
        .expect_err("a rejected large-AOI query fails after one POST");

    assert_eq!(error.debug_context["status"], 400);
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /search "));
}

#[test]
fn count_propagates_cancellation_before_any_network_request() {
    let client = AsfHttpClient::production().expect("production client builds offline");

    let error = client
        .asf_count(&params(), &|| true)
        .expect_err("cancelled count is not downgraded to unavailable");

    assert_eq!(error.code, "DL001");
}

#[test]
fn cmr_json_uses_a_bounded_get_with_encoded_parameters() {
    let server = LoopbackServer::start(vec![Reply::json(200, r#"{"feed":{"entry":[]}}"#)]);
    let client = client(&server, HttpLimits::default());
    let params = vec![
        (
            "collection_concept_id".to_string(),
            "C1214470488-ASF".to_string(),
        ),
        (
            "temporal".to_string(),
            "2024-01-01T00:00:00Z,2024-01-31T23:59:59Z".to_string(),
        ),
    ];

    let value = client
        .cmr_json(&params, &|| false)
        .expect("CMR loopback response");

    assert_eq!(value["feed"]["entry"], serde_json::json!([]));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /search?"));
    assert!(requests[0].contains("collection_concept_id=C1214470488-ASF"));
    assert!(requests[0].contains("temporal=2024-01-01T00%3A00%3A00Z%2C"));
}

#[test]
fn cmr_400_json_error_is_bounded_and_redacted_to_allowlisted_facts() {
    let body = r#"{"error_code":"INVALID_PARAMETER","field":"polygon","message":"reflected https://example.invalid/search?token=secret"}"#;
    let server = LoopbackServer::start(vec![
        Reply::json(400, body),
        Reply::json(400, body),
        Reply::json(400, body),
    ]);
    let error = client(&server, HttpLimits::default())
        .cmr_json(&[("page_num".to_owned(), "1".to_owned())], &|| false)
        .expect_err("CMR 400 is a typed error");

    assert_eq!(error.code, "ASF002");
    assert_eq!(error.debug_context["status"], 400);
    assert_eq!(
        error.debug_context["service_error_class"],
        "json_error_body"
    );
    assert_eq!(error.debug_context["safe_reason_code"], "INVALID_PARAMETER");
    assert_eq!(error.debug_context["field"], "polygon");
    assert_eq!(error.debug_context["body_bytes"], body.len());
    assert_eq!(
        error.debug_context["body_sha256"].as_str().unwrap().len(),
        64
    );
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains("reflected"));
    assert!(!diagnostic.contains("example.invalid"));
    assert!(!diagnostic.contains("token"));
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn cmr_400_html_is_not_retained() {
    let body = "<html>reflected https://example.invalid/search?q=secret&token=secret</html>";
    let server = LoopbackServer::start(vec![
        Reply::json(400, body),
        Reply::json(400, body),
        Reply::json(400, body),
    ]);
    let error = client(&server, HttpLimits::default())
        .cmr_json(&[("page_num".to_owned(), "1".to_owned())], &|| false)
        .expect_err("HTML error is not accepted");

    assert_eq!(
        error.debug_context["service_error_class"],
        "unparsed_error_body"
    );
    assert_eq!(error.debug_context["body_bytes"], body.len());
    assert!(!error.debug_context.contains_key("safe_reason_code"));
    assert!(!error.debug_context.contains_key("field"));
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains("<html>"));
    assert!(!diagnostic.contains("example.invalid"));
    assert!(!diagnostic.contains("token"));
}

#[test]
fn oversized_cmr_400_body_is_capped_before_hashing() {
    let body = "x".repeat(9 * 1024);
    let server = LoopbackServer::start(vec![
        Reply::json(400, &body),
        Reply::json(400, &body),
        Reply::json(400, &body),
    ]);
    let error = client(&server, HttpLimits::default())
        .cmr_json(&[("page_num".to_owned(), "1".to_owned())], &|| false)
        .expect_err("oversized error body is bounded");

    assert_eq!(
        error.debug_context["service_error_class"],
        "oversized_error_body"
    );
    assert_eq!(error.debug_context["safe_reason_code"], "body_too_large");
    assert_eq!(error.debug_context["body_bytes"], 8 * 1024);
    assert_eq!(
        error.debug_context["body_sha256"].as_str().unwrap().len(),
        64
    );
    let diagnostic = format!("{error:?}");
    assert!(!diagnostic.contains(&body));
}

#[test]
fn cmr_json_retries_one_transient_failure_without_changing_parameters() {
    let server = LoopbackServer::start(vec![
        Reply::json(503, "temporary-cmr-failure"),
        Reply::json(200, r#"{"feed":{"entry":[]}}"#),
    ]);
    let client = client(&server, HttpLimits::default());
    let query = vec![("page_size".to_string(), "200".to_string())];

    let result = client.cmr_json(&query, &|| false);
    if result.is_err() {
        let address = server
            .url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        drop(TcpStream::connect(address));
    }
    let value = result.expect("CMR retry succeeds");

    assert_eq!(value["feed"]["entry"], serde_json::json!([]));
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.contains("page_size=200")));
}

#[test]
fn response_size_timeout_and_status_errors_are_bounded_and_sanitized() {
    let oversized = LoopbackServer::start(vec![Reply::json(200, &"x".repeat(128))]);
    let limits = HttpLimits {
        max_json_bytes: 64,
        ..HttpLimits::default()
    };
    let error = client(&oversized, limits)
        .asf_geojson(&params(), &|| false)
        .expect_err("oversized response is rejected");
    assert_eq!(error.code, "ASF_RESPONSE_TOO_LARGE");
    assert!(!error.to_string().contains(&oversized.url));

    let mut delayed = Reply::json(200, r#"{"features":[]}"#);
    delayed.delay = Duration::from_millis(200);
    let timeout_server = LoopbackServer::start(vec![delayed.clone(), delayed]);
    let limits = HttpLimits {
        request_timeout: Duration::from_millis(50),
        ..HttpLimits::default()
    };
    let error = client(&timeout_server, limits)
        .asf_geojson(&params(), &|| false)
        .expect_err("both bounded attempts time out");
    assert_eq!(error.code, "ASF_TIMEOUT");
    let text = error.to_string();
    assert!(!text.contains("127.0.0.1"));

    let unavailable = LoopbackServer::start(Vec::new());
    let unavailable_client = client(&unavailable, HttpLimits::default());
    drop(unavailable);
    let error = unavailable_client
        .asf_geojson(&params(), &|| false)
        .expect_err("closed endpoint is a transport failure");
    assert_eq!(error.code, "ASF_TRANSPORT_FAILED");
}

#[test]
fn streaming_body_observes_cancellation_between_chunks() {
    let mut reply = Reply::json(200, &format!(r#"{{"padding":"{}"}}"#, "x".repeat(512_000)));
    reply.chunk_size = 4_096;
    reply.chunk_delay = Duration::from_millis(2);
    let server = LoopbackServer::start(vec![reply]);

    let error = client(&server, HttpLimits::default())
        .asf_geojson(&params(), &|| server.streaming_started())
        .expect_err("stream is cancelled");

    assert_eq!(error.code, "DL001");
    assert!(server.streaming_started());
    assert_eq!(server.requests().len(), 1);
}

#[test]
#[ignore = "explicit live ASF transport diagnostic"]
fn live_beijing_wkt_post_completes_within_the_production_transport_budget() {
    let source = std::env::var("INSAR_S1_LIVE_WKT_SOURCE")
        .expect("INSAR_S1_LIVE_WKT_SOURCE points to reviewed local trace evidence");
    let first = std::fs::read_to_string(source)
        .expect("live WKT evidence is readable")
        .lines()
        .next()
        .map(str::to_owned)
        .expect("live WKT evidence contains one request");
    let event: Value = serde_json::from_str(&first).expect("live WKT evidence is JSON");
    let intersects_with = event["intersects_with"]
        .as_str()
        .expect("live WKT evidence carries intersectsWith");
    let params = BTreeMap::from([
        ("dataset".to_owned(), "SENTINEL-1".to_owned()),
        ("intersectsWith".to_owned(), intersects_with.to_owned()),
        ("maxResults".to_owned(), "120".to_owned()),
        ("output".to_owned(), "geojson".to_owned()),
        ("processingLevel".to_owned(), "SLC".to_owned()),
    ]);
    let started = std::time::Instant::now();

    let value = AsfHttpClient::production()
        .expect("production ASF client initializes")
        .asf_geojson(&params, &|| false)
        .expect("reviewed Beijing WKT returns bounded GeoJSON");

    eprintln!("live ASF elapsed_ms={}", started.elapsed().as_millis());
    assert!(value.is_object());
}
