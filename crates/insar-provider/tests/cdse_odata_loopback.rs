use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::CatalogSearchQuery;
use insar_provider::cdse::loopback_client::LoopbackODataClient;
use insar_provider::cdse::query::{build_odata_request, wrap_continuation};
use serde_json::{json, Value};
use url::Url;

type ScriptedResponse = (u16, Vec<(String, String)>, String, Duration);

#[path = "support/cdse_spatial.rs"]
mod cdse_spatial;

const MAX_REQUEST_LINE_BYTES: usize = 8192;
const REQUEST_LINE_READ_TIMEOUT: Duration = Duration::from_secs(2);

fn read_request_line(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("request-line blocking-mode setup failed: {error}"))?;
    stream
        .set_read_timeout(Some(REQUEST_LINE_READ_TIMEOUT))
        .map_err(|error| format!("request-line timeout setup failed: {error}"))?;
    let mut request_line = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("request-line read failed: {error}"))?;
        if count == 0 {
            return Err("request line ended before CRLF".to_owned());
        }
        for byte in &buffer[..count] {
            if request_line.len() >= MAX_REQUEST_LINE_BYTES {
                return Err("request line exceeds the harness byte bound".to_owned());
            }
            request_line.push(*byte);
            if *byte != b'\n' {
                continue;
            }
            if request_line.len() < 2 || request_line[request_line.len() - 2] != b'\r' {
                return Err("request line is not CRLF-terminated".to_owned());
            }
            let request_line = &request_line[..request_line.len() - 2];
            if request_line.is_empty() {
                return Err("request line is empty".to_owned());
            }
            return std::str::from_utf8(request_line)
                .map(str::to_owned)
                .map_err(|_| {
                    format!(
                        "request line is not UTF-8: {}",
                        String::from_utf8_lossy(request_line)
                    )
                });
        }
    }
}

struct ScriptedServer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    running: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl ScriptedServer {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));
        let request_log = Arc::clone(&requests);
        let flag = Arc::clone(&running);
        let response_origin = origin.clone();
        let join = thread::spawn(move || {
            let mut response_index = 0;
            while flag.load(Ordering::SeqCst) && response_index < responses.len() {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                let first_line = read_request_line(&mut stream)
                    .unwrap_or_else(|reason| format!("<invalid request line: {reason}>"));
                request_log.lock().unwrap().push(first_line);
                let (status, headers, body, delay) = &responses[response_index];
                let body = body.replace(
                    "127.0.0.1:38081",
                    response_origin.trim_start_matches("http://"),
                );
                response_index += 1;
                thread::sleep(*delay);
                let reason = match status {
                    200 => "OK",
                    302 => "Found",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    503 => "Service Unavailable",
                    _ => "Error",
                };
                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(&body);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            origin,
            requests,
            running,
            join: Some(join),
        }
    }
}

impl Drop for ScriptedServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(stream) = TcpStream::connect(self.origin.trim_start_matches("http://")) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(join) = self.join.take() {
            join.join().expect("server stops deterministically");
        }
    }
}

#[test]
fn request_line_reader_reassembles_fragmented_origin_form() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback bind");
    let address = listener.local_addr().expect("loopback address");
    let writer = thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("loopback connect");
        stream
            .write_all(b"GET /odata/v1/Pro")
            .expect("first request fragment");
        thread::sleep(Duration::from_millis(2));
        stream
            .write_all(b"ducts?$top=2 HTTP/1.1\r")
            .expect("second request fragment");
        thread::sleep(Duration::from_millis(2));
        stream
            .write_all(b"\nHost: localhost\r\n\r\n")
            .expect("final request fragment");
        stream.flush().expect("flush request fragments");
    });
    let (mut stream, _) = listener.accept().expect("loopback accept");

    assert_eq!(
        read_request_line(&mut stream).expect("complete fragmented request line"),
        "GET /odata/v1/Products?$top=2 HTTP/1.1"
    );
    writer.join().expect("fragment writer joins");
}

fn fixture(name: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("tests/fixtures/catalog/cdse/sentinel2_l2a")
            .join(name),
    )
    .expect("fixture reads")
}

fn query(cursor: Option<String>) -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a", "provider_id": "copernicus.dataspace.odata", "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]},
        "cursor": cursor, "result_limit": 2,
        "filters": {"product_type":"S2MSI2A","cloud_cover_lte":4.2,"datetime":{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"}}
    }))
    .expect("query shape")
}

#[test]
fn cancellation_interrupts_wait_for_response_headers() {
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_collection_page_1.json"),
        Duration::from_millis(700),
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(2),
    )
    .unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let trigger_flag = Arc::clone(&cancelled);
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        trigger_flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();

    let error = client
        .search_all(&query(None), &|| cancelled.load(Ordering::SeqCst))
        .expect_err("cancellation must interrupt a stalled response-header wait");
    trigger.join().expect("cancellation trigger");

    assert_eq!(error.code, "CDSE_ODATA_CANCELLED");
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "client waited for delayed response headers after cancellation"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

fn resumed_query(origin: &str, skip: u64) -> CatalogSearchQuery {
    let mut query = query(None);
    let request = build_odata_request(&query).expect("base query is valid");
    let continuation = Url::parse(
        &next_link(skip, 2).replace("127.0.0.1:38081", origin.trim_start_matches("http://")),
    )
    .expect("loopback continuation is valid");
    query.cursor = Some(wrap_continuation(request.fingerprint(), &continuation).unwrap());
    query
}

fn next_link(skip: u64, top: u64) -> String {
    let source: Value = serde_json::from_str(&fixture("odata_collection_page_1.json"))
        .expect("page fixture parses");
    source["@odata.nextLink"]
        .as_str()
        .unwrap()
        .replace("$skip=2", &format!("$skip={skip}"))
        .replace("$top=2", &format!("$top={top}"))
}

fn valid_record(index: u64) -> Value {
    json!({
        "Id": format!("00000000-0000-4000-8000-{index:012}"),
        "Name": format!("S2_LOOPBACK_{index}.SAFE"),
        "Collection":{"Name":"SENTINEL-2"},
        "ContentLength": 1,
        "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
        "Online": true,
        "Attributes":[{"Name":"productType","Value":"S2MSI2A"},{"Name":"cloudCover","Value":1.0}]
    })
}

fn page_body(records: Vec<Value>, count: Option<u64>, next: Option<String>) -> String {
    let mut page = serde_json::Map::from_iter([("value".to_string(), Value::Array(records))]);
    if let Some(count) = count {
        page.insert("@odata.count".to_string(), json!(count));
    }
    if let Some(next) = next {
        page.insert("@odata.nextLink".to_string(), json!(next));
    }
    Value::Object(page).to_string()
}

#[test]
fn loopback_client_aggregates_query_scoped_pagination_and_never_uses_a_public_origin() {
    let server = ScriptedServer::start(vec![
        (
            200,
            vec![],
            fixture("odata_collection_page_1.json").replace("$top=2", "$top=100"),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            fixture("odata_collection_page_2.json"),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut all_query = query(None);
    all_query.result_limit = None;
    let response = client
        .search_all(&all_query, &|| false)
        .expect("two pages map locally");
    assert_eq!(response.products.len(), 3);
    assert_eq!(response.stats.fetched_record_count, 3);
    assert_eq!(response.stats.returned_count, 3);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].contains("GET /odata/v1/Products?"),
        "unexpected captured request line: {:?}",
        requests[0]
    );

    for endpoint in [
        "https://catalogue.dataspace.copernicus.eu/odata/v1/",
        "http://user@127.0.0.1:38081/odata/v1/",
        "http://127.0.0.1:38081/odata/v1/?query=1",
        "http://127.0.0.1:38081/odata/v1/#fragment",
    ] {
        let error = LoopbackODataClient::new(endpoint, Duration::from_secs(1))
            .err()
            .expect("non-harness endpoint is rejected");
        assert_eq!(error.code, "CDSE_ODATA_LOOPBACK_INVALID");
    }
}

#[test]
fn loopback_transport_neither_follows_redirects_nor_uses_system_proxy() {
    let redirect_target = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_empty.json"),
        Duration::ZERO,
    )]);
    let redirect_source = ScriptedServer::start(vec![(
        302,
        vec![(
            "Location".to_string(),
            format!("{}/odata/v1/Products", redirect_target.origin),
        )],
        String::new(),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", redirect_source.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let redirect_error = client.search_page(&query(None), &|| false).unwrap_err();
    assert_eq!(redirect_error.code, "CDSE_ODATA_HTTP_REDIRECT");
    assert_eq!(redirect_error.debug_context["status_code"], 302);
    assert_eq!(redirect_target.requests.lock().unwrap().len(), 0);

    let proxy = ScriptedServer::start(vec![(
        500,
        vec![],
        "proxy trap".to_string(),
        Duration::ZERO,
    )]);
    let origin = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_empty.json"),
        Duration::ZERO,
    )]);
    let prior_proxy = std::env::var_os("HTTP_PROXY");
    unsafe { std::env::set_var("HTTP_PROXY", &proxy.origin) };
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", origin.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let result = client.search_page(&query(None), &|| false);
    match prior_proxy {
        Some(value) => unsafe { std::env::set_var("HTTP_PROXY", value) },
        None => unsafe { std::env::remove_var("HTTP_PROXY") },
    }
    assert!(result.is_ok(), "no_proxy bypasses HTTP_PROXY");
    assert_eq!(origin.requests.lock().unwrap().len(), 1);
    assert_eq!(proxy.requests.lock().unwrap().len(), 0);
}

#[test]
fn loopback_client_fails_closed_for_statuses_size_and_strict_page_envelopes() {
    for body in ["{".to_string(), "{}".to_string()] {
        let server = ScriptedServer::start(vec![(200, vec![], body, Duration::ZERO)]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            client
                .search_page(&query(None), &|| false)
                .unwrap_err()
                .code,
            "CDSE_ODATA_JSON_INVALID"
        );
    }

    let count = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_count.json"),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", count.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client
            .search_page(&query(None), &|| false)
            .unwrap()
            .stats
            .matched_count,
        Some(3)
    );

    let timeout = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_empty.json"),
        Duration::from_millis(100),
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", timeout.origin),
        Duration::from_millis(5),
    )
    .unwrap();
    assert_eq!(
        client
            .search_page(&query(None), &|| false)
            .unwrap_err()
            .code,
        "CDSE_ODATA_TIMEOUT"
    );

    let oversized = format!(
        "{{\"@odata.count\":0,\"value\":[]}}{}",
        "x".repeat(1_048_576)
    );
    let server = ScriptedServer::start(vec![(200, vec![], oversized, Duration::ZERO)]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client
            .search_page(&query(None), &|| false)
            .unwrap_err()
            .code,
        "CDSE_ODATA_RESPONSE_TOO_LARGE"
    );

    let empty = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_empty.json"),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", empty.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client
            .search_page(&query(None), &|| false)
            .unwrap()
            .stats
            .matched_count,
        Some(0)
    );

    let rate: Value = serde_json::from_str(&fixture("odata_429.json")).unwrap();
    let rate_server = ScriptedServer::start(vec![(
        rate["status"].as_u64().unwrap() as u16,
        vec![(
            "Retry-After".to_string(),
            rate["headers"]["retry_after_seconds"]
                .as_str()
                .unwrap()
                .to_string(),
        )],
        rate["body"].to_string(),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", rate_server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let rate_error = client.search_page(&query(None), &|| false).unwrap_err();
    assert_eq!(rate_error.code, "CDSE_ODATA_HTTP_RATE_LIMITED");
    assert_eq!(rate_error.debug_context["status_code"], 429);
    assert_eq!(rate_error.debug_context["retry_after_seconds"], 3);

    let proxy_server = ScriptedServer::start(vec![(
        407,
        vec![(
            "Proxy-Authenticate".to_string(),
            "Basic fixture".to_string(),
        )],
        String::new(),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", proxy_server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let proxy_error = client.search_page(&query(None), &|| false).unwrap_err();
    assert_eq!(proxy_error.code, "CDSE_ODATA_HTTP_PROXY_AUTH_REQUIRED");
    assert_eq!(proxy_error.debug_context["reason"], "status");
    assert_eq!(proxy_error.debug_context["status_code"], 407);
    assert_eq!(proxy_error.debug_context.len(), 2);

    let server: Value = serde_json::from_str(&fixture("odata_5xx.json")).unwrap();
    let error_server = ScriptedServer::start(vec![(
        server["status"].as_u64().unwrap() as u16,
        vec![("Retry-After".to_string(), "61".to_string())],
        server["body"].to_string(),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", error_server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let server_error = client.search_page(&query(None), &|| false).unwrap_err();
    assert_eq!(server_error.code, "CDSE_ODATA_HTTP_SERVER");
    assert_eq!(server_error.debug_context["status_code"], 503);
    assert!(server_error
        .debug_context
        .get("retry_after_seconds")
        .is_none());
}

#[test]
fn loopback_client_preserves_only_numeric_evidence_for_generic_http_statuses() {
    const RESPONSE_BODY_SECRET: &str = "SYNTHETIC_CDSE_RESPONSE_BODY_SECRET";
    const RESPONSE_HEADER_SECRET: &str = "SYNTHETIC_CDSE_RESPONSE_HEADER_SECRET";

    for status_code in [400_u16, 405, 406, 408, 409, 413, 414, 422] {
        let server = ScriptedServer::start(vec![(
            status_code,
            vec![(
                "X-CDSE-Diagnostic".to_string(),
                RESPONSE_HEADER_SECRET.to_string(),
            )],
            RESPONSE_BODY_SECRET.to_string(),
            Duration::ZERO,
        )]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(1),
        )
        .expect("loopback client");

        let error = client.search_page(&query(None), &|| false).unwrap_err();
        assert_eq!(error.code, "CDSE_ODATA_HTTP_STATUS");
        assert_eq!(error.debug_context["reason"], "status");
        assert_eq!(error.debug_context["status_code"], status_code);
        assert_eq!(error.debug_context.len(), 2);

        let serialized = serde_json::to_string(&error).expect("error serializes");
        assert!(!serialized.contains(RESPONSE_BODY_SECRET));
        assert!(!serialized.contains(RESPONSE_HEADER_SECRET));
    }
}

#[test]
fn loopback_client_rejects_untrusted_pagination_and_cancels_between_pages() {
    let untrusted = ScriptedServer::start(vec![(
        200,
        vec![],
        fixture("odata_untrusted_next_link.json"),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", untrusted.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client
            .search_page(&query(None), &|| false)
            .unwrap_err()
            .code,
        "CDSE_ODATA_CURSOR_UNTRUSTED"
    );

    for (body, expected) in [
        (page_body(vec![], None, None), "CDSE_ODATA_PAGE_INVALID"),
        (
            page_body(vec![], Some(1), Some(next_link(1, 2))),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
        (
            page_body(
                vec![valid_record(40), valid_record(41), valid_record(42)],
                Some(3),
                None,
            ),
            "CDSE_ODATA_PAGE_INVALID",
        ),
        (
            page_body(
                vec![valid_record(43)],
                Some(1),
                Some(format!("{}&$skiptoken=also-present", next_link(1, 2))),
            ),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
        (
            page_body(vec![valid_record(44)], Some(1), Some(next_link(0, 2))),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
    ] {
        let server = ScriptedServer::start(vec![(200, vec![], body, Duration::ZERO)]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            client.search_all(&query(None), &|| false).unwrap_err().code,
            expected
        );
    }

    let mismatch = ScriptedServer::start(vec![
        (
            200,
            vec![],
            page_body(vec![valid_record(45)], Some(2), Some(next_link(1, 2))),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body(vec![valid_record(46)], Some(3), None),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", mismatch.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client.search_all(&query(None), &|| false).unwrap_err().code,
        "CDSE_ODATA_PAGE_INVALID"
    );

    let server = ScriptedServer::start(vec![
        (
            200,
            vec![],
            fixture("odata_collection_page_1.json"),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            fixture("odata_collection_page_2.json"),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let calls = AtomicUsize::new(0);
    let cancelled = || calls.fetch_add(1, Ordering::SeqCst) >= 1;
    assert_eq!(
        client
            .search_all(&query(None), &cancelled)
            .unwrap_err()
            .code,
        "CDSE_ODATA_CANCELLED"
    );
    assert_eq!(
        server.requests.lock().unwrap().len(),
        1,
        "second page was never requested"
    );
}

#[test]
fn finite_result_limit_accepts_a_bounded_final_page_without_next_link() {
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![valid_record(70), valid_record(71)], Some(5), None),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client
        .search_all(&query(None), &|| false)
        .expect("the user result limit may truncate a larger remote match set");
    assert_eq!(response.stats.matched_count, Some(5));
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.products.len(), 2);
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn loopback_client_proves_numeric_skip_progress_and_matched_count_closure() {
    for (body, expected) in [
        (
            page_body(vec![valid_record(80)], Some(2), None),
            "CDSE_ODATA_PAGE_INVALID",
        ),
        (
            page_body(vec![valid_record(81)], Some(1), Some(next_link(1, 2))),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
        (
            page_body(vec![valid_record(82)], Some(3), Some(next_link(0, 2))),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
        (
            page_body(vec![valid_record(83)], Some(3), Some(next_link(2, 2))),
            "CDSE_ODATA_CURSOR_UNTRUSTED",
        ),
    ] {
        let server = ScriptedServer::start(vec![(200, vec![], body, Duration::ZERO)]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            client.search_all(&query(None), &|| false).unwrap_err().code,
            expected
        );
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    let repeated = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![valid_record(84)], Some(3), Some(next_link(1, 2))),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", repeated.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(
        client
            .search_all(&resumed_query(&repeated.origin, 1), &|| false)
            .unwrap_err()
            .code,
        "CDSE_ODATA_CURSOR_UNTRUSTED"
    );
    assert_eq!(repeated.requests.lock().unwrap().len(), 1);

    let resumed = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![valid_record(85)], Some(3), None),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", resumed.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client
        .search_all(&resumed_query(&resumed.origin, 2), &|| false)
        .expect("resumed final page closes exactly at the matched count");
    assert_eq!(response.stats.matched_count, Some(3));
    assert_eq!(response.stats.fetched_record_count, 1);
}

#[test]
fn loopback_client_deduplicates_across_pages_and_rejects_cross_page_conflicts() {
    let exact = ScriptedServer::start(vec![
        (
            200,
            vec![],
            fixture("odata_cross_page_exact_1.json"),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            fixture("odata_cross_page_exact_2.json"),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", exact.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client.search_all(&query(None), &|| false).unwrap();
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.mapped_record_count, 2);
    assert_eq!(response.stats.exact_duplicate_record_count, 1);
    assert_eq!(response.stats.unique_provider_product_count, 1);
    assert_eq!(response.products.len(), 1);

    let conflict = ScriptedServer::start(vec![
        (
            200,
            vec![],
            fixture("odata_cross_page_conflict_1.json"),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            fixture("odata_cross_page_conflict_2.json"),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", conflict.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client.search_all(&query(None), &|| false).unwrap();
    assert_eq!(response.stats.fetched_record_count, 2);
    assert_eq!(response.stats.mapped_record_count, 0);
    assert_eq!(response.item_errors.len(), 2);
    assert!(response.products.is_empty());
}

#[test]
fn loopback_client_bounds_global_pagination_and_rejects_cycles() {
    let mut wide_query = query(None);
    wide_query.result_limit = Some(500);
    let mut responses = Vec::new();
    for page_index in 0..5_u64 {
        let records = (0..100)
            .map(|offset| valid_record(page_index * 100 + offset + 100))
            .collect();
        responses.push((
            200,
            vec![],
            page_body(
                records,
                Some(999),
                Some(next_link((page_index + 1) * 100, 100)),
            ),
            Duration::ZERO,
        ));
    }
    responses.push((200, vec![], fixture("odata_empty.json"), Duration::ZERO));
    let bounded = ScriptedServer::start(responses);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", bounded.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client.search_all(&wide_query, &|| false).unwrap();
    assert_eq!(response.stats.matched_count, Some(999));
    assert_eq!(response.stats.requested_limit, Some(500));
    assert_eq!(response.stats.fetched_record_count, 500);
    assert_eq!(response.stats.exact_matched_count, 500);
    assert_eq!(response.stats.returned_count, 500);
    assert!(!response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    assert!(response.next_cursor.is_none());
    let requests = bounded.requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    for request in requests.iter() {
        let target = request
            .strip_prefix("GET ")
            .and_then(|value| value.strip_suffix(" HTTP/1.1"))
            .expect("captured request line has an origin-form target");
        let url = Url::parse(&format!("http://loopback{target}"))
            .expect("captured request target is a valid URL");
        assert_eq!(
            url.query_pairs()
                .find(|(name, _)| name == "$top")
                .map(|(_, value)| value.into_owned())
                .as_deref(),
            Some("100")
        );
    }
    drop(requests);

    let exhausted = ScriptedServer::start(vec![
        (
            200,
            vec![],
            page_body(
                (0..100)
                    .map(|offset| valid_record(2_000 + offset))
                    .collect(),
                Some(250),
                Some(next_link(100, 100)),
            ),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body(
                (0..100)
                    .map(|offset| valid_record(2_100 + offset))
                    .collect(),
                Some(250),
                Some(next_link(200, 100)),
            ),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body(
                (0..50).map(|offset| valid_record(2_200 + offset)).collect(),
                Some(250),
                None,
            ),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", exhausted.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let response = client.search_all(&wide_query, &|| false).unwrap();
    assert_eq!(response.stats.matched_count, Some(250));
    assert_eq!(response.stats.requested_limit, Some(500));
    assert_eq!(response.stats.fetched_record_count, 250);
    assert_eq!(response.stats.exact_matched_count, 250);
    assert_eq!(response.stats.returned_count, 250);
    assert!(response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    assert!(response.next_cursor.is_none());
    assert_eq!(exhausted.requests.lock().unwrap().len(), 3);

    let cycle = ScriptedServer::start(vec![
        (
            200,
            vec![],
            fixture("odata_collection_page_1.json").replace("$top=2", "$top=100"),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            fixture("odata_collection_page_1.json").replace("$top=2", "$top=100"),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", cycle.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut all_query = query(None);
    all_query.result_limit = None;
    assert_eq!(
        client.search_all(&all_query, &|| false).unwrap_err().code,
        "CDSE_ODATA_CURSOR_UNTRUSTED"
    );
    assert_eq!(cycle.requests.lock().unwrap().len(), 2);

    let mut limited_pages = Vec::new();
    for page_index in 0..128_u64 {
        limited_pages.push((
            200,
            vec![],
            page_body(
                vec![valid_record(page_index + 700)],
                Some(999),
                Some(next_link(page_index + 1, 100)),
            ),
            Duration::ZERO,
        ));
    }
    limited_pages.push((200, vec![], fixture("odata_empty.json"), Duration::ZERO));
    let page_bounded = ScriptedServer::start(limited_pages);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", page_bounded.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut all_query = query(None);
    all_query.result_limit = None;
    let error = client.search_all(&all_query, &|| false).unwrap_err();
    assert_eq!(error.code, "CDSE_ODATA_PAGE_BUDGET_EXCEEDED");
    assert_eq!(error.debug_context["reason"], "page_budget");
    assert_eq!(page_bounded.requests.lock().unwrap().len(), 128);
}

#[test]
fn all_continues_until_cdse_remote_exhaustion_without_a_finite_surrogate() {
    let server = ScriptedServer::start(vec![
        (
            200,
            vec![],
            page_body(
                (0..100).map(valid_record).collect(),
                Some(250),
                Some(next_link(100, 100)),
            ),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body(
                (100..200).map(valid_record).collect(),
                Some(250),
                Some(next_link(200, 100)),
            ),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body((200..250).map(valid_record).collect(), Some(250), None),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let mut all_query = query(None);
    all_query.result_limit = None;

    let response = client.search_all(&all_query, &|| false).unwrap();

    assert_eq!(response.stats.requested_limit, None);
    assert_eq!(response.products.len(), 250);
    assert_eq!(response.stats.returned_count, 250);
    assert!(response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    assert!(response.next_cursor.is_none());
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

/// #22 B slice 3(a): generated 100-record pages up to `pages`; the last
/// page keeps a next link unless the catalogue is `exhausted`.
fn generated_cdse_pages(available: u64, pages: u64, exhausted: bool) -> Vec<ScriptedResponse> {
    (0..pages)
        .map(|page| {
            let start = page * 100;
            let end = (start + 100).min(available);
            let last = page + 1 == pages;
            let next = (!last || !exhausted).then(|| next_link(end, 100));
            (
                200,
                vec![],
                page_body(
                    (start..end).map(valid_record).collect(),
                    Some(available),
                    next,
                ),
                Duration::ZERO,
            )
        })
        .collect()
}

#[test]
fn ten_thousand_target_keeps_n_and_leaves_exhaustion_unproven_or_proven() {
    for requested in [10_000, 10_001] {
        let server = ScriptedServer::start(generated_cdse_pages(10_001, 100, false));
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(5),
        )
        .unwrap();
        let mut capped = query(None);
        capped.result_limit = Some(requested);
        let response = client.search_all(&capped, &|| false).unwrap();
        assert_eq!(response.stats.requested_limit, Some(10_000));
        assert_eq!(response.products.len(), 10_000);
        assert_eq!(response.stats.matched_count, Some(10_001));
        assert!(!response.stats.remote_exhausted);
        assert_eq!(server.requests.lock().unwrap().len(), 100);
    }

    let server = ScriptedServer::start(generated_cdse_pages(10_000, 100, true));
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(5),
    )
    .unwrap();
    let mut exact = query(None);
    exact.result_limit = Some(10_000);
    let response = client.search_all(&exact, &|| false).unwrap();
    assert_eq!(response.products.len(), 10_000);
    assert!(response.stats.remote_exhausted);
}

#[test]
fn explicit_target_above_the_ceiling_has_the_ceiling_fingerprint() {
    let mut at_cap = query(None);
    at_cap.result_limit = Some(10_000);
    let mut above = at_cap.clone();
    above.result_limit = Some(10_001);
    assert_eq!(
        build_odata_request(&at_cap).unwrap().fingerprint(),
        build_odata_request(&above).unwrap().fingerprint()
    );
}
