use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use insar_core::ContractError;
use insar_provider::orbit_download::{OrbitHttpClient, OrbitResolverEndpoint};

const POEORB: &str =
    "S1A_OPER_AUX_POEORB_OPOD_20240102T120000_V20240101T225942_20240103T005942.EOF";
const RESORB: &str =
    "S1A_OPER_AUX_RESORB_OPOD_20240102T120000_V20240101T225942_20240103T005942.EOF";

struct Server {
    endpoint: String,
    request: Arc<Mutex<String>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(file_name: &str, status: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&request);
        let file_name = file_name.to_string();
        let status = status.to_string();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            *captured.lock().unwrap() = read_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Disposition: attachment; filename=\"{file_name}\"\r\nContent-Range: bytes 0-0/4096\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx"
            );
            let _ = stream.write_all(response.as_bytes());
        });
        Self {
            endpoint: format!("http://{address}/scene/"),
            request,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut buffer).unwrap_or(0);
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn client(server: &Server) -> OrbitHttpClient {
    OrbitHttpClient::with_test_endpoint(
        OrbitResolverEndpoint::loopback_for_tests(&server.endpoint).unwrap(),
    )
    .unwrap()
}

fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn acquired_days_before(now: DateTime<Utc>, days: i64) -> String {
    (now - ChronoDuration::days(days)).to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn category_of(error: &ContractError) -> &str {
    error
        .debug_context
        .get("category")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn assert_message_is_safe(error: &ContractError, endpoint: &str) {
    let message = error.user_message.to_ascii_lowercase();
    assert!(
        !message.contains("http"),
        "user message must not contain a URL: {}",
        error.user_message
    );
    assert!(
        !message.contains(endpoint),
        "user message must not contain the endpoint: {}",
        error.user_message
    );
    assert_eq!(
        error.debug_context.len(),
        1,
        "debug context must contain only the category"
    );
}

#[test]
fn production_endpoint_is_fixed_and_poeorb_resolution_is_bounded() {
    assert_eq!(
        OrbitResolverEndpoint::production().as_str(),
        "https://s1-orbits.asf.alaska.edu/scene/"
    );
    let server = Server::start(POEORB, "206 Partial Content");

    let resolved = client(&server)
        .resolve(
            "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234",
            None,
            &|| false,
        )
        .unwrap();

    assert_eq!(resolved.file_name, POEORB);
    assert_eq!(resolved.expected_size, Some(4096));
    assert_eq!(
        resolved.item_id,
        "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234"
    );
    assert!(!format!("{resolved:?}").contains(&server.endpoint));
    let request = server.request.lock().unwrap();
    assert!(request.starts_with("GET /scene/S1A_IW_SLC"));
    assert!(request.to_ascii_lowercase().contains("range: bytes=0-0"));
}

#[test]
fn missing_poeorb_http_failure_is_classified_by_scene_age() {
    let now = fixed_now();
    for (age_days, code, category, recent, late) in [
        (0_i64, "ORB002", "not_published", true, false),
        (19_i64, "ORB002", "not_published", true, false),
        (20_i64, "ORB002", "not_published", false, false),
        (25_i64, "ORB002", "not_published", false, true),
        (59_i64, "ORB002", "not_published", false, true),
        (60_i64, "ORB003", "wait_limit_exceeded", false, false),
        (61_i64, "ORB003", "wait_limit_exceeded", false, false),
    ] {
        let server = Server::start(POEORB, "404 Not Found");
        let error = client(&server)
            .with_test_clock(now)
            .resolve(
                "S1A_TEST_SCENE",
                Some(&acquired_days_before(now, age_days)),
                &|| false,
            )
            .unwrap_err();
        assert_eq!(error.code, code, "scene age {age_days} days");
        assert_eq!(category_of(&error), category, "scene age {age_days} days");
        assert_eq!(
            error.user_message.contains("采集时间较近"),
            recent,
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_eq!(
            error.user_message.contains("发布偏晚"),
            late,
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_eq!(
            error.user_message.contains("上限"),
            code == "ORB003",
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_message_is_safe(&error, &server.endpoint);
    }
}

#[test]
fn resorb_only_response_is_classified_by_scene_age() {
    let now = fixed_now();
    for (age_days, code, category, late) in [
        (3_i64, "ORB002", "not_published_non_poeorb", false),
        (30_i64, "ORB002", "not_published_non_poeorb", true),
        (70_i64, "ORB003", "wait_limit_exceeded_non_poeorb", false),
    ] {
        let server = Server::start(RESORB, "206 Partial Content");
        let error = client(&server)
            .with_test_clock(now)
            .resolve(
                "S1A_TEST_SCENE",
                Some(&acquired_days_before(now, age_days)),
                &|| false,
            )
            .unwrap_err();
        assert_eq!(error.code, code, "scene age {age_days} days");
        assert_eq!(category_of(&error), category, "scene age {age_days} days");
        assert!(
            error.user_message.contains("非 POEORB"),
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_eq!(
            error.user_message.contains("发布偏晚"),
            late,
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_eq!(
            error.user_message.contains("上限"),
            code == "ORB003",
            "scene age {age_days} days: {}",
            error.user_message
        );
        assert_message_is_safe(&error, &server.endpoint);
    }
}

#[test]
fn transport_malformed_and_unknown_age_failures_remain_orb001() {
    let now = fixed_now();
    let young = acquired_days_before(now, 3);

    // Transport failure (connection refused) stays ORB001 even for a young scene.
    let refused = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let endpoint = format!("http://{refused}/scene/");
    let error = OrbitHttpClient::with_test_endpoint(
        OrbitResolverEndpoint::loopback_for_tests(&endpoint).unwrap(),
    )
    .unwrap()
    .with_test_clock(now)
    .resolve("S1A_TEST_SCENE", Some(&young), &|| false)
    .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert_eq!(category_of(&error), "connect");

    // Malformed response file name stays ORB001 even for a young scene.
    let server = Server::start("not-an-orbit-file.txt", "206 Partial Content");
    let error = client(&server)
        .with_test_clock(now)
        .resolve("S1A_TEST_SCENE", Some(&young), &|| false)
        .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert_eq!(category_of(&error), "invalid_orbit_name");
    assert_message_is_safe(&error, &server.endpoint);

    // Server and access failures are not "not published yet": they stay ORB001
    // even for a young scene, so they are never scheduled as a wait.
    for status in [
        "500 Internal Server Error",
        "403 Forbidden",
        "503 Service Unavailable",
    ] {
        let server = Server::start(POEORB, status);
        let error = client(&server)
            .with_test_clock(now)
            .resolve("S1A_TEST_SCENE", Some(&young), &|| false)
            .unwrap_err();
        assert_eq!(error.code, "ORB001", "{status}");
        assert_eq!(category_of(&error), "unavailable", "{status}");
        assert_message_is_safe(&error, &server.endpoint);
    }

    // No acquisition time: age cannot be classified, keep the legacy ORB001.
    let server = Server::start(POEORB, "404 Not Found");
    let error = client(&server)
        .with_test_clock(now)
        .resolve("S1A_TEST_SCENE", None, &|| false)
        .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert_eq!(category_of(&error), "unavailable");

    // Unparseable acquisition time: same legacy classification.
    let server = Server::start(POEORB, "404 Not Found");
    let error = client(&server)
        .with_test_clock(now)
        .resolve("S1A_TEST_SCENE", Some("not-a-timestamp"), &|| false)
        .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert_eq!(category_of(&error), "unavailable");
}

#[test]
fn resorb_without_age_and_cancellation_keep_legacy_classification() {
    let server = Server::start(RESORB, "206 Partial Content");
    let error = client(&server)
        .resolve("S1A_TEST_SCENE", None, &|| false)
        .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert!(error.user_message.contains("POEORB"));

    let production = OrbitHttpClient::production().unwrap();
    let error = production
        .resolve("S1A_TEST_SCENE", None, &|| true)
        .unwrap_err();
    assert_eq!(error.code, "DL001");
}

#[test]
fn explicit_resorb_request_accepts_resorb_and_rejects_missing_or_mismatched_type() {
    let now = fixed_now();
    let young = acquired_days_before(now, 3);

    // Explicit manual RESORB request resolves AUX_RESORB EOF.
    let server = Server::start(RESORB, "206 Partial Content");
    let resolved = client(&server)
        .with_test_clock(now)
        .resolve_with_orbit_type(
            "S1A_TEST_SCENE",
            Some(&young),
            insar_core::OrbitProductType::RESORB,
            &|| false,
        )
        .unwrap();
    assert_eq!(resolved.file_name, RESORB);

    // Missing RESORB fails as ORB001 (never schedules POEORB auto-recheck).
    let missing = Server::start(RESORB, "404 Not Found");
    let error = client(&missing)
        .with_test_clock(now)
        .resolve_with_orbit_type(
            "S1A_TEST_SCENE",
            Some(&young),
            insar_core::OrbitProductType::RESORB,
            &|| false,
        )
        .unwrap_err();
    assert_eq!(error.code, "ORB001");
    assert_eq!(category_of(&error), "resorb_not_returned");
}
