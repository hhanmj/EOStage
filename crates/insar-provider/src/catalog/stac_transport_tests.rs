use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::*;

#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    path: String,
    headers: String,
    body: Vec<u8>,
}
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    header_delay: Duration,
    body_delay: Duration,
    content_length: Option<usize>,
    chunked: bool,
}
impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            headers: vec![("Content-Type".into(), "application/geo+json".into())],
            body: serde_json::to_vec(&value).unwrap(),
            header_delay: Duration::ZERO,
            body_delay: Duration::ZERO,
            content_length: None,
            chunked: false,
        }
    }
}

struct Server {
    origin: String,
    calls: Arc<AtomicUsize>,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let c = calls.clone();
        let r = recorded.clone();
        let s = stop.clone();
        let handle = thread::spawn(move || {
            let mut replies = replies.into_iter();
            while !s.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("local fixture accept failed: {e}"),
                };
                // Windows may inherit the listener's nonblocking mode. The
                // accepted stream uses bounded blocking reads independently.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request = read_request(&mut stream);
                r.lock().unwrap().push(request);
                c.fetch_add(1, Ordering::SeqCst);
                let reply = replies.next().unwrap_or_else(|| {
                    let mut fallback = Reply::json(json!({"unexpected_request":true}));
                    fallback.status = 500;
                    fallback
                });
                if !wait_delay(reply.header_delay, &s) {
                    break;
                }
                let mut header = format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", reply.status);
                for (key, value) in reply.headers {
                    header.push_str(&format!("{key}: {value}\r\n"));
                }
                if reply.chunked {
                    header.push_str("Transfer-Encoding: chunked\r\n");
                } else {
                    header.push_str(&format!(
                        "Content-Length: {}\r\n",
                        reply.content_length.unwrap_or(reply.body.len())
                    ));
                }
                header.push_str("\r\n");
                if stream.write_all(header.as_bytes()).is_err() {
                    continue;
                }
                let _ = stream.flush();
                if !wait_delay(reply.body_delay, &s) {
                    break;
                }
                if reply.chunked {
                    let _ = write!(stream, "{:x}\r\n", reply.body.len());
                    let _ = stream.write_all(&reply.body);
                    let _ = stream.write_all(b"\r\n0\r\n\r\n");
                } else {
                    let _ = stream.write_all(&reply.body);
                }
            }
        });
        Self {
            origin,
            calls,
            recorded,
            stop,
            thread: Some(handle),
        }
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let result = self.thread.take().unwrap().join();
        // Preserve the original assertion instead of aborting the entire test
        // binary with a second panic during unwinding. A server failure still
        // fails an otherwise successful test.
        if !thread::panicking() {
            assert!(result.is_ok(), "local fixture server failed");
        }
    }
}
fn wait_delay(delay: Duration, stop: &AtomicBool) -> bool {
    let until = Instant::now() + delay;
    while Instant::now() < until {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    !stop.load(Ordering::SeqCst)
}
fn read_request(stream: &mut TcpStream) -> Recorded {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let end = loop {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0 && bytes.len() + n <= 64 * 1024);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    assert!(length <= 32 * 1024);
    while bytes.len() - end < length {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    let first = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>();
    Recorded {
        method: first[0].into(),
        path: first[1].into(),
        headers,
        body: bytes[end..end + length].to_vec(),
    }
}

fn route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac".into(),
        endpoint: "https://catalog.example.test/stac/v1/".into(),
        search_path: "search".into(),
        page_cap: 2,
        accepted_stac_versions: vec!["1.1.0".into()],
    }
}
fn binding() -> StacCollectionBinding {
    StacCollectionBinding {
        logical_dataset: "optical.fixture.l2".into(),
        provider_id: "fixture.stac".into(),
        native_collection_id: "fixture-l2".into(),
        collection_profile_id: "fixture.stac.l2.v1".into(),
        identity_rule_id: "fixture.stac.id.v1".into(),
        filename_rule_id: "fixture.stac.filename.v1".into(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind: DataSourceKind::Sentinel2,
        product_kind: ProductKind::OpticalScene,
        production_enabled: false,
        product_id_property: None,
        display_name_property: None,
        queryables: vec![],
        asset_role_bindings: vec![StacAssetRoleBinding {
            stac_asset_key: "data".into(),
            role: AssetRole::Primary,
            required: true,
            allowed_hosts: vec!["data.example.test".into()],
        }],
    }
}
fn geometry() -> Value {
    json!({"type":"Polygon","coordinates":[[[120,20],[121,20],[121,21],[120,20]]]})
}
fn query(limit: Option<u64>) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: "optical.fixture.l2".into(),
        provider_id: "fixture.stac".into(),
        native_collection_id: "fixture-l2".into(),
        aoi_geojson: Some(geometry().as_object().unwrap().clone()),
        cursor: None,
        result_limit: limit,
        filters: Map::new(),
    }
}
fn feature(id: &str) -> Value {
    json!({"type":"Feature","stac_version":"1.1.0","id":id,"collection":"fixture-l2",
        "geometry":geometry(),"properties":{"datetime":"2026-09-11T00:00:00Z"},
        "assets":{"data":{"href":format!("https://data.example.test/{id}.tif"),"roles":["data"]}},"links":[]})
}
fn page(ids: &[&str], next: Option<Value>) -> Value {
    json!({"type":"FeatureCollection","features":ids.iter().map(|id| feature(id)).collect::<Vec<_>>(),
        "numberReturned":ids.len(),"links":next.into_iter().collect::<Vec<_>>()})
}
fn get_next(token: &str) -> Value {
    json!({"rel":"next","href":format!("/stac/v1/search?page={token}")})
}
fn client(server: &Server) -> StacLoopbackClient {
    StacLoopbackClient::new_for_test(
        route(),
        binding(),
        &server.origin,
        StacTransportLimits::default(),
    )
    .unwrap()
}

#[test]
fn search_budget_accepts_one_hundred_pages_for_ten_thousand_results() {
    let limits = StacTransportLimits {
        max_pages: 100,
        max_records: 10_000,
        ..StacTransportLimits::default()
    };
    assert!(limits.validate().is_ok());
}

#[test]
fn separate_parts_share_one_page_budget() {
    let server = Server::new(vec![Reply::json(page(&["a"], None))]);
    let mut c = client(&server);
    c.transport.limits.max_pages = 1;
    let mut budget = c.transport.search_budget();
    c.transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap();
    let error = c
        .transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap_err();
    assert_eq!(error.code, "STAC_LIMIT_EXCEEDED");
    assert_eq!(error.debug_context["reason"], "pages");
    assert_eq!(server.count(), 1);
}

#[test]
fn separate_parts_share_one_record_budget() {
    let server = Server::new(vec![
        Reply::json(page(&["a"], None)),
        Reply::json(page(&["b"], None)),
    ]);
    let mut c = client(&server);
    c.transport.limits.max_records = 1;
    let mut budget = c.transport.search_budget();
    c.transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap();
    let error = c
        .transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap_err();
    assert_eq!(error.code, "STAC_LIMIT_EXCEEDED");
    assert_eq!(error.debug_context["reason"], "records");
    assert_eq!(budget.records, 1);
    assert_eq!(server.count(), 2);
}

#[test]
fn separate_parts_share_one_response_byte_budget() {
    let value = page(&["a"], None);
    let size = serde_json::to_vec(&value).unwrap().len();
    let server = Server::new(vec![Reply::json(value.clone()), Reply::json(value)]);
    let mut c = client(&server);
    c.transport.limits.max_page_bytes = size;
    c.transport.limits.max_total_bytes = size * 2 - 1;
    let mut budget = c.transport.search_budget();
    c.transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap();
    let error = c
        .transport
        .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
        .unwrap_err();
    assert_eq!(error.code, "STAC_LIMIT_EXCEEDED");
    assert_eq!(error.debug_context["reason"], "response_bytes");
    assert_eq!(budget.total_bytes, size);
    assert_eq!(server.count(), 2);
}

#[test]
fn separate_parts_do_not_reset_deadline_or_cancellation() {
    for cancelled in [false, true] {
        let server = Server::new(vec![Reply::json(page(&["a"], None))]);
        let c = client(&server);
        let mut budget = c.transport.search_budget();
        c.transport
            .search_with_mapper_in_budget(&query(Some(1)), &|| false, &mut budget, |_, p| Ok(p))
            .unwrap();
        budget.deadline = Instant::now();
        let error = c
            .transport
            .search_with_mapper_in_budget(&query(Some(1)), &|| cancelled, &mut budget, |_, p| Ok(p))
            .unwrap_err();
        assert_eq!(
            error.code,
            if cancelled {
                "STAC_SEARCH_CANCELLED"
            } else {
                "STAC_TIMEOUT"
            }
        );
        assert_eq!(server.count(), 1);
    }
}

#[test]
fn mapper_observes_every_valid_record_before_result_truncation() {
    let server = Server::new(vec![Reply::json(page(&["a", "b"], None))]);
    let c = client(&server);
    let mut seen = Vec::new();
    let response = c
        .transport
        .search_with_mapper(&query(Some(1)), &|| false, |_, p| {
            seen.push(p.product_id.clone());
            Ok(p)
        })
        .unwrap();
    assert_eq!(seen, vec!["a", "b"]);
    assert_eq!(response.products.len(), 1);
    assert_eq!(response.stats.mapped_record_count, 2);
    assert_eq!(response.stats.exact_duplicate_record_count, 0);
}

#[test]
fn scientific_mapper_cannot_bypass_raw_stac_validation_or_change_identity() {
    for mutation in 0..4 {
        let mut value = page(&["a"], None);
        match mutation {
            0 => {
                value["features"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("stac_version");
            }
            1 => value["features"][0]["stac_version"] = json!("0.9.0"),
            2 => value["features"][0]["properties"]["access_token"] = json!("synthetic-forbidden"),
            _ => value["features"][0]["geometry"] = Value::Null,
        }
        let server = Server::new(vec![Reply::json(value)]);
        let calls = AtomicUsize::new(0);
        let error = client(&server)
            .transport
            .search_with_mapper(&query(Some(1)), &|| false, |_, product| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(product)
            })
            .unwrap_err();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "invalid raw item reached mapper: {mutation}"
        );
        assert_eq!(error.code, "STAC_RESPONSE_INVALID");
        assert_eq!(server.count(), 1);
    }
    let server = Server::new(vec![Reply::json(page(&["a"], None))]);
    let error = client(&server)
        .transport
        .search_with_mapper(&query(Some(1)), &|| false, |_, mut product| {
            product.product_id = "different-item".to_owned();
            Ok(product)
        })
        .unwrap_err();
    assert_eq!(error.code, "STAC_IDENTITY_MISMATCH");
}

#[test]
fn real_socket_get_paging_preserves_request_and_unknown_total() {
    let server = Server::new(vec![
        Reply::json(page(&["b"], Some(get_next("2")))),
        Reply::json(page(&["a"], None)),
    ]);
    let response = client(&server).search(&query(Some(2)), &|| false).unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(
        response
            .products
            .iter()
            .map(|p| p.product_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.matched_count, None);
    assert!(response.stats.remote_exhausted && response.stats.working_set_satisfied);
    assert!(response.next_cursor.is_none() && !response.stats.truncated);
    assert!(response
        .products
        .iter()
        .flat_map(|p| &p.assets)
        .all(|a| a.access_descriptor.is_none()));
    response.validate().unwrap();
    let requests = server.recorded.lock().unwrap();
    assert_eq!(requests[0].method, "POST");
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["intersects"], geometry());
    assert_eq!(body["limit"], 2);
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].path, "/stac/v1/search?page=2");
    for r in requests.iter() {
        let h = r.headers.to_ascii_lowercase();
        assert!(
            !h.contains("authorization:")
                && !h.contains("cookie:")
                && !h.contains("proxy-authorization:")
        );
    }
}

#[test]
fn post_continuation_body_is_transmitted_without_merging_or_debug_leak() {
    let next = json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":false,"body":{"token":"synthetic-page-2"}});
    let server = Server::new(vec![
        Reply::json(page(&["a"], Some(next))),
        Reply::json(page(&["b"], None)),
    ]);
    let c = client(&server);
    c.search(&query(None), &|| false).unwrap();
    let req = server.recorded.lock().unwrap();
    assert_eq!(req[1].method, "POST");
    assert_eq!(
        serde_json::from_slice::<Value>(&req[1].body).unwrap(),
        json!({"token":"synthetic-page-2"})
    );
    assert!(!format!("{c:?}").contains("synthetic-page-2"));
}

fn dated_query(limit: Option<u64>) -> CatalogSearchQuery {
    let mut query = query(limit);
    query.filters.insert(
        "datetime".into(),
        json!({"start":"2026-09-01T00:00:00Z","end":"2026-09-30T00:00:00Z"}),
    );
    query
}
fn body(request: &Recorded) -> Value {
    serde_json::from_slice(&request.body).unwrap()
}

#[test]
fn reviewed_sort_sources_are_earth_search_and_planetary_computer_only() {
    assert!(stac_sorts_newest_first("element84.earth-search"));
    assert!(stac_sorts_newest_first("microsoft.planetary-computer"));
    assert!(!stac_sorts_newest_first("fixture.stac"));
    assert!(!stac_sorts_newest_first("copernicus.dataspace.stac"));
}

#[test]
fn newest_first_source_posts_sortby_on_a_dated_search() {
    let server = Server::new(vec![Reply::json(page(&["a"], None))]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    c.search(&dated_query(Some(1)), &|| false).unwrap();
    let requests = server.recorded.lock().unwrap();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        body(&requests[0])["sortby"],
        json!([{"field":"properties.datetime","direction":"desc"}])
    );
    assert_eq!(
        body(&requests[0])["datetime"],
        "2026-09-01T00:00:00Z/2026-09-30T00:00:00Z"
    );
}

#[test]
fn non_conforming_or_undated_search_gets_no_sortby() {
    // The fixture provider does not advertise the sort extension.
    let server = Server::new(vec![Reply::json(page(&["a"], None))]);
    client(&server)
        .search(&dated_query(Some(1)), &|| false)
        .unwrap();
    assert!(body(&server.recorded.lock().unwrap()[0])
        .get("sortby")
        .is_none());
    // An undated search has no time order to request.
    let server = Server::new(vec![Reply::json(page(&["a"], None))]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    c.search(&query(Some(1)), &|| false).unwrap();
    assert!(body(&server.recorded.lock().unwrap()[0])
        .get("sortby")
        .is_none());
}

#[test]
fn newest_first_get_continuation_carries_the_get_sortby_form() {
    let next = json!({"rel":"next","href":"/stac/v1/search?datetime=2026-09-01T00:00:00Z/2026-09-30T00:00:00Z&page=2"});
    let server = Server::new(vec![
        Reply::json(page(&["a"], Some(next))),
        Reply::json(page(&["b"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    c.search(&dated_query(None), &|| false).unwrap();
    let requests = server.recorded.lock().unwrap();
    assert_eq!(requests[1].method, "GET");
    let url = Url::parse(&format!("http://loopback{}", requests[1].path)).unwrap();
    let sort = url
        .query_pairs()
        .filter(|(key, _)| key == "sortby")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    assert_eq!(sort, ["-properties.datetime"]);
}

#[test]
fn merge_true_post_next_link_is_merged_onto_the_original_request() {
    let next = json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":true,
        "body":{"token":"next:synthetic-2","limit":1}});
    let server = Server::new(vec![
        Reply::json(page(&["a"], Some(next))),
        Reply::json(page(&["b"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let response = c.search(&dated_query(None), &|| false).unwrap();
    assert_eq!(response.products.len(), 2);
    let requests = server.recorded.lock().unwrap();
    let first = body(&requests[0]);
    let second = body(&requests[1]);
    assert_eq!(requests[1].method, "POST");
    assert_eq!(second["token"], "next:synthetic-2");
    assert_eq!(second["limit"], 1);
    for key in ["collections", "intersects", "datetime", "sortby"] {
        assert_eq!(
            second[key], first[key],
            "{key} is kept from the original request"
        );
    }
    assert!(!format!("{c:?}").contains("synthetic-2"));
}

#[test]
fn newest_first_cap_keeps_the_first_records_in_server_order() {
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(get_next("2")))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let response = c.search(&dated_query(Some(3)), &|| false).unwrap();
    let ids = response
        .products
        .iter()
        .map(|product| product.product_id.as_str())
        .collect::<Vec<_>>();
    // The provider's newest-first order decides which records survive the
    // cap; the returned list keeps the adapter's product-id order.
    assert_eq!(ids, ["b", "c", "d"]);
    assert_eq!(response.stats.fetched_record_count, 4);
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.transport_part_count, 1);
}

#[test]
fn undated_multi_page_cap_keeps_product_id_truncation() {
    // No datetime, so no sortby was sent: server order means nothing and the
    // previous product-id truncation stays, even on a sort-capable source.
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(get_next("2")))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let response = c.search(&query(Some(3)), &|| false).unwrap();
    let ids = response
        .products
        .iter()
        .map(|product| product.product_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["a", "b", "c"]);
    assert!(body(&server.recorded.lock().unwrap()[0])
        .get("sortby")
        .is_none());
}

#[test]
fn a_differing_provider_sortby_is_not_treated_as_newest_first() {
    let next = json!({"rel":"next","href":"/stac/v1/search?datetime=2026-09-01T00:00:00Z/2026-09-30T00:00:00Z&sortby=%2Bproperties.datetime&page=2"});
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(next))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let response = c.search(&dated_query(Some(3)), &|| false).unwrap();
    let ids = response
        .products
        .iter()
        .map(|product| product.product_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["a", "b", "c"]);
    assert!(server.recorded.lock().unwrap()[1]
        .path
        .contains("sortby=%2Bproperties.datetime"));
}

#[test]
fn merge_uses_original_body_instead_of_carrying_stale_previous_page_fields() {
    let first_next = json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":true,
        "body":{"token":"next:2","stale_page_hint":"page-1-only"}});
    let second_next = json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":true,
        "body":{"token":"next:3"}});
    let server = Server::new(vec![
        Reply::json(page(&["a"], Some(first_next))),
        Reply::json(page(&["b"], Some(second_next))),
        Reply::json(page(&["c"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    c.search(&dated_query(None), &|| false).unwrap();
    let requests = server.recorded.lock().unwrap();
    let original = body(&requests[0]);
    let second = body(&requests[1]);
    let third = body(&requests[2]);
    assert_eq!(second["stale_page_hint"], "page-1-only");
    assert_eq!(third["token"], "next:3");
    assert!(third.get("stale_page_hint").is_none());
    let mut expected = original.as_object().unwrap().clone();
    expected.insert("token".into(), json!("next:3"));
    assert_eq!(third, Value::Object(expected));
}

#[test]
fn finite_target_stops_without_fetching_next_page_and_total_is_separate() {
    let mut p = page(&["a"], Some(get_next("2")));
    p["numberMatched"] = json!(2500);
    let server = Server::new(vec![Reply::json(p)]);
    let r = client(&server).search(&query(Some(1)), &|| false).unwrap();
    assert_eq!(server.count(), 1);
    assert_eq!(r.products.len(), 1);
    assert_eq!(r.stats.matched_count, Some(2500));
    assert!(!r.stats.remote_exhausted && r.stats.working_set_satisfied);
}

#[test]
fn exhausted_empty_response_is_success_not_unknown_failure() {
    let server = Server::new(vec![Reply::json(page(&[], None))]);
    let r = client(&server).search(&query(None), &|| false).unwrap();
    assert!(r.products.is_empty() && r.stats.remote_exhausted);
    assert_eq!(r.stats.matched_count, None);
}

#[test]
fn later_page_failure_never_returns_partial_success_or_retries() {
    let mut bad = Reply::json(json!({"secret":"synthetic-response-marker"}));
    bad.status = 503;
    let server = Server::new(vec![Reply::json(page(&["a"], Some(get_next("2")))), bad]);
    let e = client(&server)
        .search(&query(Some(2)), &|| false)
        .unwrap_err();
    assert_eq!(e.code, "STAC_SERVER_ERROR");
    assert_eq!(server.count(), 2);
    assert!(!format!("{e:?}").contains("synthetic-response-marker"));
}

#[test]
fn status_classes_do_not_probe_credentials_retry_or_follow_redirects() {
    for (status, code) in [
        (302, "STAC_REDIRECT_REJECTED"),
        (401, "STAC_ACCESS_DENIED"),
        (403, "STAC_ACCESS_DENIED"),
        (429, "STAC_RATE_LIMITED"),
        (500, "STAC_SERVER_ERROR"),
        (404, "STAC_ITEM_NOT_FOUND"),
    ] {
        let mut p = Reply::json(json!({"password":"synthetic-never-log"}));
        p.status = status;
        p.headers.push((
            "Location".into(),
            "https://never-contact.example.test/".into(),
        ));
        p.headers
            .push(("Set-Cookie".into(), "synthetic-cookie".into()));
        let s = Server::new(vec![p]);
        let e = client(&s).search(&query(None), &|| false).unwrap_err();
        assert_eq!(e.code, code);
        assert_eq!(s.count(), 1);
        assert!(!format!("{e:?}").contains("synthetic-never-log"));
    }
}

#[test]
fn untrusted_next_links_are_rejected_before_second_socket_request() {
    for next in [
        json!({"rel":"next","href":"https://evil.example.test/stac/v1/search"}),
        json!({"rel":"next","href":"/stac/v1/logout"}),
        json!({"rel":"next","href":"/stac/v1/search#fragment"}),
        json!({"rel":"next","href":"/stac/v1/search?access_token=synthetic"}),
        json!({"rel":"next","href":"/stac/v1/search","method":2}),
        json!({"rel":"next","href":"/stac/v1/search","method":"DELETE"}),
        json!({"rel":"next","href":"/stac/v1/search","headers":{}}),
        json!({"rel":"next","href":"/stac/v1/search?page=2","merge":true}),
        json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":"yes","body":{}}),
    ] {
        let s = Server::new(vec![Reply::json(page(&["a"], Some(next)))]);
        assert_eq!(
            client(&s).search(&query(None), &|| false).unwrap_err().code,
            "STAC_CURSOR_INVALID"
        );
        assert_eq!(s.count(), 1);
    }
}

#[test]
fn duplicate_products_changed_totals_and_repeated_cursors_fail_closed() {
    let s = Server::new(vec![
        Reply::json(page(&["a"], Some(get_next("2")))),
        Reply::json(page(&["a"], None)),
    ]);
    assert_eq!(
        client(&s).search(&query(None), &|| false).unwrap_err().code,
        "STAC_IDENTITY_MISMATCH"
    );
    let mut a = page(&["a"], Some(get_next("2")));
    a["numberMatched"] = json!(10);
    let mut b = page(&["b"], None);
    b["numberMatched"] = json!(11);
    let s = Server::new(vec![Reply::json(a), Reply::json(b)]);
    assert_eq!(
        client(&s).search(&query(None), &|| false).unwrap_err().code,
        "STAC_RESPONSE_INVALID"
    );
    let s = Server::new(vec![
        Reply::json(page(&["a"], Some(get_next("2")))),
        Reply::json(page(&["b"], Some(get_next("2")))),
    ]);
    assert_eq!(
        client(&s).search(&query(None), &|| false).unwrap_err().code,
        "STAC_CURSOR_INVALID"
    );
    assert_eq!(s.count(), 2);
}

#[test]
fn pages_and_records_have_independent_hard_budgets() {
    let s = Server::new(vec![Reply::json(page(&["a"], Some(get_next("2"))))]);
    let limits = StacTransportLimits {
        max_pages: 1,
        ..Default::default()
    };
    let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, limits).unwrap();
    assert_eq!(
        c.search(&query(None), &|| false).unwrap_err().code,
        "STAC_LIMIT_EXCEEDED"
    );
    assert_eq!(s.count(), 1);
    let s = Server::new(vec![Reply::json(page(&["a", "b"], None))]);
    let limits = StacTransportLimits {
        max_records: 1,
        ..Default::default()
    };
    let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, limits).unwrap();
    assert_eq!(
        c.search(&query(None), &|| false).unwrap_err().code,
        "STAC_LIMIT_EXCEEDED"
    );
}

#[test]
fn response_budgets_cover_content_length_chunked_and_aggregate_bytes() {
    for chunked in [false, true] {
        let mut reply = Reply::json(page(&["a"], None));
        reply.chunked = chunked;
        let s = Server::new(vec![reply]);
        let limits = StacTransportLimits {
            max_page_bytes: 64,
            max_total_bytes: 128,
            ..Default::default()
        };
        let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, limits).unwrap();
        assert_eq!(
            c.search(&query(None), &|| false).unwrap_err().code,
            "STAC_LIMIT_EXCEEDED"
        );
    }
    let a = Reply::json(page(&["a"], Some(get_next("2"))));
    let size = a.body.len();
    let s = Server::new(vec![a, Reply::json(page(&["b"], None))]);
    let limits = StacTransportLimits {
        max_page_bytes: size + 32,
        max_total_bytes: size + 32,
        ..Default::default()
    };
    let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, limits).unwrap();
    assert_eq!(
        c.search(&query(None), &|| false).unwrap_err().code,
        "STAC_LIMIT_EXCEEDED"
    );
    assert_eq!(s.count(), 2);
}

#[test]
fn malformed_json_mime_encoding_and_counts_are_not_accepted() {
    for fault in 0..5 {
        let mut reply = Reply::json(page(&["a"], None));
        match fault {
            0 => reply.body = b"not json: synthetic-error".to_vec(),
            1 => reply.headers = vec![("Content-Type".into(), "text/html".into())],
            2 => reply
                .headers
                .push(("Content-Encoding".into(), "gzip".into())),
            3 => {
                let mut p = page(&["a"], None);
                p["numberReturned"] = json!(2);
                reply = Reply::json(p);
            }
            _ => {
                let mut p = page(&["a"], None);
                p["numberMatched"] = json!(0);
                reply = Reply::json(p);
            }
        }
        let s = Server::new(vec![reply]);
        let e = client(&s).search(&query(None), &|| false).unwrap_err();
        assert_eq!(e.code, "STAC_RESPONSE_INVALID");
        assert!(!format!("{e:?}").contains("synthetic-error"));
    }
}

#[test]
fn truncated_http_body_is_a_network_error_not_partial_json() {
    let mut reply = Reply::json(page(&["a"], None));
    reply.content_length = Some(reply.body.len() + 100);
    let s = Server::new(vec![reply]);
    assert_eq!(
        client(&s).search(&query(None), &|| false).unwrap_err().code,
        "STAC_NETWORK_ERROR"
    );
}

fn short_limits() -> StacTransportLimits {
    StacTransportLimits {
        request_timeout: Duration::from_millis(150),
        overall_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}

#[test]
fn stalled_headers_and_body_obey_request_timeout_without_retry() {
    for body in [false, true] {
        let mut reply = Reply::json(page(&["a"], None));
        if body {
            reply.body_delay = Duration::from_millis(600);
        } else {
            reply.header_delay = Duration::from_millis(600);
        }
        let s = Server::new(vec![reply]);
        let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, short_limits())
            .unwrap();
        let started = Instant::now();
        let e = c.search(&query(None), &|| false).unwrap_err();
        assert_eq!(e.code, "STAC_TIMEOUT");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(s.count(), 1);
    }
}

#[test]
fn overall_deadline_spans_multiple_requests() {
    let mut a = Reply::json(page(&["a"], Some(get_next("2"))));
    a.header_delay = Duration::from_millis(100);
    let mut b = Reply::json(page(&["b"], None));
    b.header_delay = Duration::from_millis(600);
    let s = Server::new(vec![a, b]);
    let limits = StacTransportLimits {
        request_timeout: Duration::from_millis(250),
        overall_timeout: Duration::from_millis(250),
        ..Default::default()
    };
    let c = StacLoopbackClient::new_for_test(route(), binding(), &s.origin, limits).unwrap();
    let started = Instant::now();
    assert_eq!(
        c.search(&query(None), &|| false).unwrap_err().code,
        "STAC_TIMEOUT"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(s.count(), 2);
}

#[test]
fn cancellation_before_dispatch_and_during_stall_never_fetches_another_page() {
    let s = Server::new(vec![]);
    assert_eq!(
        client(&s).search(&query(None), &|| true).unwrap_err().code,
        "STAC_SEARCH_CANCELLED"
    );
    assert_eq!(s.count(), 0);
    let mut p = Reply::json(page(&["a"], Some(get_next("2"))));
    p.header_delay = Duration::from_millis(600);
    let s = Server::new(vec![p]);
    let c =
        StacLoopbackClient::new_for_test(route(), binding(), &s.origin, short_limits()).unwrap();
    let e = c.search(&query(None), &|| s.count() > 0).unwrap_err();
    assert_eq!(e.code, "STAC_SEARCH_CANCELLED");
    assert_eq!(s.count(), 1);
}

#[test]
fn cancellation_during_a_blocked_body_read_wins_over_timeout_error() {
    let mut reply = Reply::json(page(&["a"], None));
    reply.body_delay = Duration::from_millis(600);
    let s = Server::new(vec![reply]);
    let calls = s.calls.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        thread::sleep(Duration::from_millis(40));
        flag.store(true, Ordering::SeqCst);
    });
    let c =
        StacLoopbackClient::new_for_test(route(), binding(), &s.origin, short_limits()).unwrap();
    let e = c
        .search(&query(None), &|| cancelled.load(Ordering::SeqCst))
        .unwrap_err();
    thread.join().unwrap();
    assert_eq!(e.code, "STAC_SEARCH_CANCELLED");
    assert_eq!(s.count(), 1);
}

#[test]
fn origin_admission_and_query_rejection_happen_without_any_network() {
    for origin in [
        "https://example.test/",
        "http://localhost:1234/",
        "http://192.0.2.1:1234/",
        "http://127.0.0.1:1234/admin",
        "http://user@127.0.0.1:1234/",
        "http://127.0.0.1:0/",
    ] {
        assert!(
            StacLoopbackClient::new_for_test(route(), binding(), origin, Default::default())
                .is_err()
        );
    }
    let s = Server::new(vec![]);
    let c = client(&s);
    let mut q = query(None);
    q.provider_id = "asf.catalog".into();
    assert!(c.search(&q, &|| false).is_err());
    let mut q = query(None);
    q.cursor = Some("untrusted".into());
    assert!(c.search(&q, &|| false).is_err());
    let mut q = query(None);
    q.filters.insert("secret".into(), json!(true));
    assert!(c.search(&q, &|| false).is_err());
    assert_eq!(s.count(), 0);
}

/// #22 B slice 3(a) fixtures: generated, never committed JSON.
fn cap_route() -> StacRouteProfile {
    StacRouteProfile {
        page_cap: 100,
        ..route()
    }
}

fn cap_limits() -> StacTransportLimits {
    StacTransportLimits {
        max_pages: 128,
        max_records: insar_core::SEARCH_SAFETY_CAP as usize,
        max_page_bytes: 1024 * 1024,
        max_total_bytes: 64 * 1024 * 1024,
        ..StacTransportLimits::default()
    }
}

/// `available` generated items in pages of 100; the last page has no next
/// link only when `exhausted`.
fn generated_pages(available: usize, exhausted: bool) -> Vec<Reply> {
    let ids = (0..available)
        .map(|index| format!("item-{index:05}"))
        .collect::<Vec<_>>();
    let chunks = ids.chunks(100).collect::<Vec<_>>();
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            let refs = chunk.iter().map(String::as_str).collect::<Vec<_>>();
            let last = index + 1 == chunks.len();
            let next = (!last || !exhausted).then(|| get_next(&(index + 2).to_string()));
            Reply::json(page(&refs, next))
        })
        .collect()
}

fn cap_client(server: &Server) -> StacLoopbackClient {
    StacLoopbackClient::new_for_test(cap_route(), binding(), &server.origin, cap_limits()).unwrap()
}

#[test]
fn ten_thousand_target_over_ten_thousand_and_one_keeps_n_without_proving_exhaustion() {
    for requested in [10_000, 10_001] {
        let server = Server::new(generated_pages(10_001, true));
        let response = cap_client(&server)
            .search(&query(Some(requested)), &|| false)
            .unwrap();
        assert_eq!(server.count(), 100, "the 101st page is never requested");
        assert_eq!(response.products.len(), 10_000);
        assert_eq!(response.stats.requested_limit, Some(10_000));
        assert_eq!(response.stats.returned_count, 10_000);
        // The target stopped the walk: exhaustion is not proven, so the
        // frozen batch is possibly incomplete.
        assert!(!response.stats.remote_exhausted);
        assert!(response.stats.working_set_satisfied && !response.stats.truncated);
    }
}

#[test]
fn exactly_ten_thousand_available_and_exhausted_is_complete() {
    let server = Server::new(generated_pages(10_000, true));
    let response = cap_client(&server)
        .search(&query(Some(10_000)), &|| false)
        .unwrap();
    assert_eq!(server.count(), 100);
    assert_eq!(response.products.len(), 10_000);
    assert!(response.stats.remote_exhausted);
}

#[test]
fn byte_or_time_budget_never_returns_a_short_ten_thousand_result() {
    // A total byte budget below what 10,000 records need fails closed with
    // a typed limit; it never returns the pages read so far as a result.
    let server = Server::new(generated_pages(10_001, true));
    let limits = StacTransportLimits {
        max_total_bytes: 2 * 1024 * 1024,
        ..cap_limits()
    };
    let client =
        StacLoopbackClient::new_for_test(cap_route(), binding(), &server.origin, limits).unwrap();
    let error = client.search(&query(Some(10_000)), &|| false).unwrap_err();
    assert_eq!(error.code, "STAC_LIMIT_EXCEEDED");
    assert_eq!(error.debug_context["reason"], "response_bytes");
    assert!(server.count() < 100);

    // The overall deadline likewise fails closed instead of truncating.
    let server = Server::new(generated_pages(10_001, true));
    let c = cap_client(&server);
    let mut budget = c.transport.search_budget();
    budget.deadline = Instant::now() + Duration::from_millis(300);
    let mut slow_mapper_calls = 0_usize;
    let error = c
        .transport
        .search_with_mapper_in_budget(&query(Some(10_000)), &|| false, &mut budget, |_, p| {
            slow_mapper_calls += 1;
            if slow_mapper_calls.is_multiple_of(100) {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(p)
        })
        .unwrap_err();
    assert_eq!(error.code, "STAC_TIMEOUT");
    assert!(server.count() < 100);
}

#[test]
fn explicit_target_above_the_ceiling_is_fingerprinted_as_the_ceiling() {
    let at_cap = build_stac_search_request(&cap_route(), &binding(), &query(Some(10_000))).unwrap();
    let above = build_stac_search_request(&cap_route(), &binding(), &query(Some(10_001))).unwrap();
    assert_eq!(at_cap.query_fingerprint, above.query_fingerprint);
    assert_eq!(at_cap.body, above.body);
    // Defaults stay unset; the route's page size bounds the request.
    let default = build_stac_search_request(&cap_route(), &binding(), &query(None)).unwrap();
    assert_eq!(default.body.as_ref().unwrap()["limit"], json!(100));
}

// #22 B slice 3(b): the streaming paging loop must keep the whole-response
// adapter's cap semantics and statistics while handing each page's mapped
// products to the sink immediately.

#[test]
fn streaming_matches_whole_response_for_zero_one_and_multi_page_dated_searches() {
    let make_pages = |label: &str| -> Vec<Reply> {
        match label {
            "empty" => vec![Reply::json(page(&[], None))],
            "single" => vec![Reply::json(page(&["a"], None))],
            "multi-page under limit" | "multi-page capped" => vec![
                Reply::json(page(&["d", "c"], Some(get_next("2")))),
                Reply::json(page(&["b", "a"], None)),
            ],
            _ => unreachable!("fixed labels"),
        }
    };
    for (label, limit) in [
        ("empty", Some(1)),
        ("single", Some(1)),
        ("multi-page under limit", Some(4)),
        ("multi-page capped", Some(3)),
    ] {
        // Whole response for the expected order.
        let server = Server::new(make_pages(label));
        let mut c = client(&server);
        c.transport.newest_first = true;
        let expected = c.search(&dated_query(limit), &|| false).unwrap();

        // Streaming must observe the same transport order and cap.
        let server = Server::new(make_pages(label));
        let mut c = client(&server);
        c.transport.newest_first = true;
        let mut seen: Vec<Vec<String>> = Vec::new();
        let (collection, stats) = c
            .transport
            .search_streaming(
                &dated_query(limit),
                &|| false,
                &mut c.transport.search_budget(),
                |_, p| Ok(p),
                &mut |chunk: &[ProviderProduct]| {
                    seen.push(
                        chunk
                            .iter()
                            .map(|p| p.product_id.clone())
                            .collect::<Vec<_>>(),
                    );
                    Ok(())
                },
            )
            .unwrap();
        let flattened = seen.concat();
        let expected_ids = expected
            .products
            .iter()
            .map(|p| p.product_id.clone())
            .collect::<Vec<_>>();
        // The sink sees the transport's own order (server order), not the
        // whole-response adapter's item-ID order; the two views agree as
        // multisets and the caller re-freezes the final order.
        assert_eq!(
            flattened.iter().collect::<std::collections::BTreeSet<_>>(),
            expected_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>(),
            "{label}: same kept products"
        );
        assert_eq!(flattened.len(), expected_ids.len(), "{label}");
        assert_eq!(collection, expected.collection, "{label}");
        let mut stats = stats;
        stats.elapsed_ms = 0;
        let mut expected_stats = expected.stats.clone();
        expected_stats.elapsed_ms = 0;
        assert_eq!(stats, expected_stats, "{label}");
        // A newest-first stream keeps the server order, not item-ID order.
        if label == "multi-page capped" {
            assert_eq!(flattened, vec!["d", "c", "b"], "{label}: server order");
        }
    }
}

#[test]
fn streaming_bounds_the_observed_chunk_to_one_page_and_rejects_larger_sinks() {
    // 250 items over 3 pages (page_cap 100): each observed chunk is at most
    // one page; nothing accumulates across pages.
    let ids = (0..250).map(|i| format!("item-{i:03}")).collect::<Vec<_>>();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let pages: Vec<Reply> = refs
        .chunks(100)
        .enumerate()
        .map(|(index, chunk)| {
            let last = index == 2;
            Reply::json(page(
                chunk,
                (!last).then(|| get_next(&(index + 2).to_string())),
            ))
        })
        .collect();
    let server = Server::new(pages);
    let mut c =
        StacLoopbackClient::new_for_test(cap_route(), binding(), &server.origin, cap_limits())
            .unwrap();
    c.transport.newest_first = true;
    let mut max_chunk = 0_usize;
    let mut total = 0_usize;
    let dated = dated_query(Some(251));
    let (_collection, stats) = c
        .transport
        .search_streaming(
            &dated,
            &|| false,
            &mut c.transport.search_budget(),
            |_, p| Ok(p),
            &mut |chunk: &[ProviderProduct]| {
                max_chunk = max_chunk.max(chunk.len());
                total += chunk.len();
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(max_chunk, 100, "one page per chunk");
    assert_eq!(total, 250);
    assert_eq!(stats.returned_count, 250);
}

#[test]
fn streaming_undated_search_fails_closed_with_a_typed_reason() {
    // No datetime: no sortby, so the product-ID cap cannot be decided while
    // streaming. The transport must fail closed before flushing any page.
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(get_next("2")))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let c = client(&server);
    let mut flushed = 0_usize;
    let error = c
        .transport
        .search_streaming(
            &query(Some(3)),
            &|| false,
            &mut c.transport.search_budget(),
            |_, p| Ok(p),
            &mut |_chunk: &[ProviderProduct]| {
                flushed += 1;
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "STAC_RESPONSE_INVALID");
    assert_eq!(error.debug_context["reason"], "streaming_cap_order");
    assert_eq!(flushed, 0, "no page may be flushed before failing closed");
}

#[test]
fn streaming_cancellation_before_each_page_never_flushes_the_next_chunk() {
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(get_next("2")))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let cancel = AtomicBool::new(false);
    let mut chunks = 0_usize;
    let error = c
        .transport
        .search_streaming(
            &dated_query(Some(4)),
            &|| cancel.load(Ordering::Acquire),
            &mut c.transport.search_budget(),
            |_, p| Ok(p),
            &mut |_chunk: &[ProviderProduct]| {
                chunks += 1;
                // Cancel right after the first page is flushed.
                cancel.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "STAC_SEARCH_CANCELLED");
    assert_eq!(chunks, 1, "only the first page reaches the sink");
}

#[test]
fn streaming_later_page_failure_never_returns_partial_stats() {
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(get_next("2")))),
        Reply {
            status: 500,
            ..Reply::json(page(&["b", "a"], None))
        },
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let error = c
        .transport
        .search_streaming(
            &dated_query(Some(4)),
            &|| false,
            &mut c.transport.search_budget(),
            |_, p| Ok(p),
            &mut |_chunk: &[ProviderProduct]| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "STAC_SERVER_ERROR");
}

/// #22 B slice 3(b) review fix 3 (transport half): a page-2 continuation
/// that states a different sort mid-search fails the strict stream closed
/// with the typed `streaming_cap_order` reason AFTER pages have already been
/// flushed. This is the transport-level trigger for the whole-response retry;
/// the dispatcher-side retry test lives in insar-service.
#[test]
fn streaming_late_sort_change_fails_closed_after_flushed_pages() {
    // Page 1: two products, continuation to page 2 (merge:false POST body
    // carrying a DIFFERENT sortby, which strict streaming cannot honor).
    // Page 2: two more products, no continuation.
    let re_sorted_next = json!({
        "rel":"next","href":"/stac/v1/search","method":"POST","merge":false,
        "body":{"sortby":[{"field":"properties.datetime","direction":"asc"}]}
    });
    let server = Server::new(vec![
        Reply::json(page(&["d", "c"], Some(re_sorted_next))),
        Reply::json(page(&["b", "a"], None)),
    ]);
    let mut c = client(&server);
    c.transport.newest_first = true;
    let mut flushed: Vec<String> = Vec::new();
    let mut flushed_pages = 0_usize;
    let error = c
        .transport
        .search_streaming(
            &dated_query(Some(4)),
            &|| false,
            &mut c.transport.search_budget(),
            |_, p| Ok(p),
            &mut |chunk: &[ProviderProduct]| {
                flushed_pages += 1;
                flushed.extend(chunk.iter().map(|product| product.product_id.clone()));
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "STAC_RESPONSE_INVALID");
    assert_eq!(error.debug_context["reason"], "streaming_cap_order");
    assert_eq!(
        flushed_pages, 1,
        "page 1 must be flushed before failing closed"
    );
    assert_eq!(
        flushed,
        vec!["d".to_owned(), "c".to_owned()],
        "the flushed prefix is page 1's server order"
    );
}
