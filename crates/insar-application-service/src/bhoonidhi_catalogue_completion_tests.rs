//! Exercise the real provider response against the unchanged application gate.
use super::*;
use insar_provider::bhoonidhi::{
    build_search_request, BhoonidhiAccessTokenProvider, BhoonidhiAuthError, BhoonidhiCatalogLimits,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

#[path = "../../insar-provider/src/bhoonidhi/fixtures.rs"]
mod fixtures;

struct FixtureToken;
impl BhoonidhiAccessTokenProvider for FixtureToken {
    fn access_token(&self) -> Result<zeroize::Zeroizing<String>, BhoonidhiAuthError> {
        Ok(zeroize::Zeroizing::new("fixture-only-token".to_owned()))
    }
}

fn receive_request(stream: &mut TcpStream) {
    // Windows accepts inherit listener properties; reads use the bounded blocking timeout.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0 && bytes.len() + count <= 16 * 1024);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            assert!(headers.starts_with("POST /data/search HTTP/1.1"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
}

fn query(limit: Option<u64>) -> CatalogSearchQuery {
    serde_json::from_value(serde_json::json!({
        "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
        "provider_id": BHOONIDHI_PROVIDER_ID,
        "native_collection_id": "NISAR_SSAR_RSLC",
        "aoi_geojson": null, "cursor": null, "result_limit": limit,
        "filters": {"online": "Y"}
    }))
    .unwrap()
}

fn search_with_next(
    limit: Option<u64>,
    page_only: bool,
) -> (CatalogSearchResponse, CatalogSearchQuery) {
    let query = query(limit);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/data/search", listener.local_addr().unwrap());
    let mut next_body = build_search_request(&query).unwrap().body().clone();
    let observed_next: Value = serde_json::from_str(fixtures::NEXT_BODY_JSON).unwrap();
    next_body.insert("fields".to_owned(), observed_next["fields"].clone());
    next_body.insert(
        "token".to_owned(),
        Value::String("fixture-next-page".to_owned()),
    );
    let feature: Value = serde_json::from_str(fixtures::CATALOGUE_FEATURE_JSON).unwrap();
    let first = serde_json::json!({
        "type": "FeatureCollection", "features": [feature],
        "links": [{"rel": "next", "href": endpoint, "method": "POST", "body": next_body}]
    });
    let expected_requests = if page_only || limit == Some(1) { 1 } else { 2 };
    let server = thread::spawn(move || {
        for index in 0..expected_requests {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "bounded loopback request missing"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("loopback accept: {error}"),
                }
            };
            receive_request(&mut stream);
            let body = if index == 0 {
                first.clone()
            } else {
                serde_json::json!({"type": "FeatureCollection", "features": [], "links": []})
            }
            .to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let client = BhoonidhiCatalogClient::with_endpoint_for_tests(
        &endpoint,
        FixtureToken,
        BhoonidhiCatalogLimits::default(),
    )
    .unwrap();
    let response = if page_only {
        client.search_page(&query, &|| false)
    } else {
        client.search(&query, &|| false)
    };
    server.join().unwrap();
    (response.unwrap(), query)
}

#[test]
fn bhoonidhi_finite_working_set_is_complete_without_claiming_remote_exhaustion() {
    let (response, query) = search_with_next(Some(1), false);
    assert_eq!(response.products.len(), 1);
    assert_eq!(response.stats.request_count, 1);
    assert!(!response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    assert!(
        !response.stats.truncated,
        "one requested item is not an incomplete working set"
    );
    assert!(response.next_cursor.is_none());
    validate_complete_bhoonidhi_catalogue_response(&response, &query).unwrap();
}

#[test]
fn bhoonidhi_page_api_retains_continuation_and_cannot_be_frozen_as_complete() {
    let (response, query) = search_with_next(Some(1), true);
    assert!(response.stats.truncated);
    assert!(response.next_cursor.is_some());
    assert!(!response.stats.remote_exhausted);
    assert_eq!(
        validate_complete_bhoonidhi_catalogue_response(&response, &query)
            .unwrap_err()
            .code(),
        "CATALOGUE_RESPONSE_INCOMPLETE"
    );
}

#[test]
fn bhoonidhi_underfilled_and_all_queries_follow_next_until_remote_exhaustion() {
    for limit in [Some(2), None] {
        let (response, query) = search_with_next(limit, false);
        assert_eq!(response.stats.request_count, 2);
        assert_eq!(response.products.len(), 1);
        assert!(response.stats.remote_exhausted);
        assert!(!response.stats.truncated);
        assert!(response.next_cursor.is_none());
        validate_complete_bhoonidhi_catalogue_response(&response, &query).unwrap();
    }
}
