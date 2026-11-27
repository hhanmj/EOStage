use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;

struct Server {
    origin: String,
    requests: Arc<AtomicUsize>,
    request_lines: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(body: String, delay: Duration, delayed_body: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}/odata/v1/", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let request_lines = Arc::new(Mutex::new(Vec::new()));
        let (count, flag, lines) = (requests.clone(), stop.clone(), request_lines.clone());
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                // Windows accepted sockets can inherit the listener's mode.
                // Read the complete bounded header before simulating latency.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut header_bytes = Vec::new();
                let mut buf = [0; 1024];
                while !header_bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).expect("loopback request header");
                    assert!(n > 0 && header_bytes.len() + n <= 8192);
                    header_bytes.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&header_bytes);
                lines
                    .lock()
                    .unwrap()
                    .push(head.lines().next().unwrap_or_default().to_owned());
                count.fetch_add(1, Ordering::Release);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if !delayed_body {
                    thread::sleep(delay);
                }
                if socket.write_all(header.as_bytes()).is_ok() {
                    if delayed_body {
                        thread::sleep(delay);
                    }
                    let _ = socket.write_all(body.as_bytes());
                }
            }
        });
        Self {
            origin,
            requests,
            request_lines,
            stop,
            thread: Some(thread),
        }
    }
    fn engine(&self) -> ODataHttpEngine {
        ODataHttpEngine::new(
            Url::parse(&self.origin).unwrap(),
            Duration::from_secs(5),
            ProviderNetworkPolicy::direct(),
        )
        .unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn query() -> CatalogSearchQuery {
    serde_json::from_value(json!({"logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":"copernicus.dataspace.odata","native_collection_id":"SENTINEL-2",
        "aoi_geojson":{"type":"MultiPolygon","coordinates":[
            [[[97.0,39.0],[99.0,39.0],[99.0,41.0],[97.0,41.0],[97.0,39.0]]],
            [[[123.0,49.0],[125.0,49.0],[125.0,51.0],[123.0,51.0],[123.0,49.0]]]
        ]},"cursor":null,"result_limit":null,"filters":{"product_type":"S2MSI2A"}
    }))
    .unwrap()
}

#[test]
fn spatial_page_and_byte_budgets_are_shared_across_request_parts() {
    let body = json!({"@odata.count":0,"value":[]}).to_string();
    for bytes in [false, true] {
        let server = Server::new(body.clone(), Duration::ZERO, false);
        let engine = server.engine();
        let start = Instant::now();
        let mut budget = SpatialBudget::new(start, Duration::from_secs(3));
        if bytes {
            budget.bytes_left = body.len() as u64 + 1;
        } else {
            budget.pages_left = 1;
        }
        let error = engine
            .search_spatial_in_budget(&query(), &|| false, start, &mut budget)
            .unwrap_err();
        assert_eq!(
            error.code,
            if bytes {
                "CDSE_ODATA_RESPONSE_TOO_LARGE"
            } else {
                "CDSE_ODATA_PAGE_BUDGET_EXCEEDED"
            }
        );
        assert_eq!(
            server.requests.load(Ordering::Acquire),
            if bytes { 2 } else { 1 }
        );
    }
}

#[test]
fn spatial_record_budget_counts_raw_records_including_aoi_rejections() {
    let record = json!({"Id":"00000000-0000-4000-8000-000000000001","Name":"S2_BUDGET.SAFE",
        "Collection":{"Name":"SENTINEL-2"},"ContentLength":1,"ContentDate":{"Start":"2026-01-01T00:00:00Z"},
        "Online":true,"Attributes":[{"Name":"productType","Value":"S2MSI2A"}],
        "GeoFootprint":{"type":"Polygon","coordinates":[[[110.0,44.0],[111.0,44.0],[111.0,45.0],[110.0,44.0]]]}});
    let server = Server::new(
        json!({"@odata.count":1,"value":[record]}).to_string(),
        Duration::ZERO,
        false,
    );
    let engine = server.engine();
    let start = Instant::now();
    let mut budget = SpatialBudget::new(start, Duration::from_secs(3));
    budget.records_left = 1;
    let error = engine
        .search_spatial_in_budget(&query(), &|| false, start, &mut budget)
        .unwrap_err();
    assert_eq!(error.code, "CDSE_ODATA_RESULT_LIMIT_EXCEEDED");
    assert_eq!(server.requests.load(Ordering::Acquire), 1);
}

#[test]
fn spatial_deadline_covers_all_parts_headers_and_body_waits() {
    for delayed_body in [false, true] {
        let server = Server::new(
            json!({"@odata.count":0,"value":[]}).to_string(),
            Duration::from_millis(180),
            delayed_body,
        );
        let engine = server.engine();
        let start = Instant::now();
        let mut budget = SpatialBudget::new(start, Duration::from_millis(300));
        let error = engine
            .search_spatial_in_budget(&query(), &|| false, start, &mut budget)
            .unwrap_err();
        assert_eq!(error.code, "CDSE_ODATA_TIMEOUT");
        assert!(
            start.elapsed() < Duration::from_millis(650),
            "deadline must not restart per tile"
        );
        assert_eq!(server.requests.load(Ordering::Acquire), 2);
    }
}

#[test]
fn spatial_cancellation_interrupts_headers_and_body_without_another_part() {
    for delayed_body in [false, true] {
        let server = Server::new(
            json!({"@odata.count":0,"value":[]}).to_string(),
            Duration::from_millis(250),
            delayed_body,
        );
        let engine = server.engine();
        let start = Instant::now();
        let mut budget = SpatialBudget::new(start, Duration::from_secs(3));
        let error = engine
            .search_spatial_in_budget(
                &query(),
                &|| start.elapsed() > Duration::from_millis(60),
                start,
                &mut budget,
            )
            .unwrap_err();
        assert_eq!(
            error.code, "CDSE_ODATA_CANCELLED",
            "{error:?}, delayed_body={delayed_body}"
        );
        assert!(start.elapsed() < Duration::from_millis(220));
        assert_eq!(server.requests.load(Ordering::Acquire), 1);
    }
}

#[test]
fn spatial_parts_skip_rectangles_outside_the_aoi_and_page_the_largest_overlap_first() {
    // Small western component, large north-eastern component: the planner
    // covers the envelope with four rectangles, two of which hold no AOI.
    let mut query = query();
    query.aoi_geojson = Some(
        json!({"type":"MultiPolygon","coordinates":[
            [[[97.0,39.0],[98.0,39.0],[98.0,40.0],[97.0,40.0],[97.0,39.0]]],
            [[[121.0,46.0],[125.0,46.0],[125.0,51.0],[121.0,51.0],[121.0,46.0]]]
        ]})
        .as_object()
        .unwrap()
        .clone(),
    );
    let record = json!({"Id":"00000000-0000-4000-8000-000000000001","Name":"S2_ORDER.SAFE",
        "Collection":{"Name":"SENTINEL-2"},"ContentLength":1,"ContentDate":{"Start":"2026-01-01T00:00:00Z"},
        "Online":true,"Attributes":[{"Name":"productType","Value":"S2MSI2A"}],
        "GeoFootprint":{"type":"Polygon","coordinates":[[[122.0,47.0],[123.0,47.0],[123.0,48.0],[122.0,48.0],[122.0,47.0]]]}});
    let server = Server::new(
        json!({"@odata.count":1,"value":[record]}).to_string(),
        Duration::ZERO,
        false,
    );
    let engine = server.engine();
    let start = Instant::now();
    let mut budget = SpatialBudget::new(start, Duration::from_secs(3));
    let response = engine
        .search_spatial_in_budget(&query, &|| false, start, &mut budget)
        .unwrap();
    assert_eq!(
        server.requests.load(Ordering::Acquire),
        2,
        "the two rectangles without AOI overlap are never requested"
    );
    let lines = server.request_lines.lock().unwrap().clone();
    assert!(
        lines[0].contains("111+45"),
        "largest overlap first: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("97+39"),
        "then the western part: {}",
        lines[1]
    );
    assert_eq!(response.stats.transport_part_count, 2);
    assert_eq!(response.products.len(), 1);
    assert!(response.stats.remote_exhausted);
}

#[test]
fn spatial_search_deadline_is_one_bounded_multiple_of_the_request_timeout() {
    assert_eq!(SPATIAL_DEADLINE_REQUEST_MULTIPLE, 4);
    let server = Server::new(
        json!({"@odata.count":0,"value":[]}).to_string(),
        Duration::ZERO,
        false,
    );
    let engine = server.engine();
    let started = Instant::now();
    let budget = SpatialBudget::new(
        started,
        engine
            .timeout
            .saturating_mul(SPATIAL_DEADLINE_REQUEST_MULTIPLE),
    );
    assert_eq!(budget.deadline - started, Duration::from_secs(20));
}

#[test]
fn production_spatial_search_fetches_three_skip_pages_concurrently_without_count() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}/odata/v1/", listener.local_addr().unwrap());
    let server_origin = origin.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let request_lines = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (count, flag, lines) = (requests.clone(), stop.clone(), request_lines.clone());
    let server_thread = thread::spawn(move || {
        let mut workers = Vec::new();
        while !flag.load(Ordering::Acquire) {
            let Ok((mut socket, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(2));
                continue;
            };
            let count = count.clone();
            let lines = lines.clone();
            let server_origin = server_origin.clone();
            workers.push(thread::spawn(move || {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut header_bytes = Vec::new();
                let mut buf = [0_u8; 1024];
                while !header_bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).expect("loopback request header");
                    assert!(n > 0 && header_bytes.len() + n <= 8192);
                    header_bytes.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&header_bytes);
                let first_line = head.lines().next().unwrap_or_default().to_owned();
                lines.lock().unwrap().push(first_line.clone());
                count.fetch_add(1, Ordering::Release);

                let (skip, record_count, next_skip) =
                    if first_line.contains("$skip=200") || first_line.contains("%24skip=200") {
                        (200_u64, 80_u64, None)
                    } else if first_line.contains("$skip=100") || first_line.contains("%24skip=100")
                    {
                        (100_u64, 100_u64, Some(200_u64))
                    } else {
                        (0_u64, 100_u64, Some(100_u64))
                    };

                let records = (0..record_count)
                    .map(|idx| {
                        let seq = skip + idx + 1;
                        json!({
                            "Id": format!("00000000-0000-4000-8000-{seq:012}"),
                            "Name": format!("S2A_MSIL2A_20260101T000000_N0511_R001_T50RKU_{seq:04}.SAFE"),
                            "Collection": {"Name": "SENTINEL-2"},
                            "ContentLength": 1024,
                            "ContentDate": {"Start": "2026-01-01T00:00:00Z"},
                            "Online": true,
                            "Attributes": [{"Name": "productType", "Value": "S2MSI2A"}],
                            "GeoFootprint": {
                                "type": "Polygon",
                                "coordinates": [[[117.0, 31.6], [117.3, 31.6], [117.3, 31.9], [117.0, 31.9], [117.0, 31.6]]]
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                let mut page = json!({ "value": records });
                if let Some(next_skip) = next_skip {
                    page.as_object_mut().unwrap().insert(
                        "@odata.nextLink".to_owned(),
                        json!(format!("{server_origin}Products?$skip={next_skip}")),
                    );
                }
                let body = page.to_string();
                thread::sleep(Duration::from_millis(150));
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(body.as_bytes());
            }));
        }
        for w in workers {
            let _ = w.join();
        }
    });

    let mut ring = (0..64)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 64.0;
            let lon = ((117.2 + 0.5 * angle.cos()) * 10_000.0).round() / 10_000.0;
            let lat = ((31.8 + 0.5 * angle.sin()) * 10_000.0).round() / 10_000.0;
            json!([lon, lat])
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    let mut dense_query = query();
    dense_query.aoi_geojson = Some(
        json!({"type": "Polygon", "coordinates": [ring]})
            .as_object()
            .unwrap()
            .clone(),
    );
    dense_query.result_limit = Some(250);

    let engine = ODataHttpEngine::new_for_route(
        Url::parse(&origin).unwrap(),
        Duration::from_secs(5),
        ProviderNetworkPolicy::direct(),
        RouteUse::Production,
    )
    .unwrap();

    let started = Instant::now();
    let response = engine.search_all(&dense_query, &|| false).unwrap();
    let elapsed = started.elapsed();

    stop.store(true, Ordering::Release);
    server_thread.join().unwrap();

    assert!(
        elapsed < Duration::from_millis(380),
        "3 concurrent 150ms pages should complete in < 380ms (actual: {elapsed:?})"
    );
    assert_eq!(requests.load(Ordering::Acquire), 3);
    let captured_lines = request_lines.lock().unwrap().clone();
    assert_eq!(captured_lines.len(), 3);
    assert!(
        captured_lines
            .iter()
            .all(|line| !line.contains("$count") && !line.contains("%24count")),
        "production spatial search requests must omit $count: {captured_lines:?}"
    );
    assert_eq!(response.products.len(), 250);
    assert_eq!(response.stats.matched_count, None);
    assert_eq!(response.stats.request_count, 3);
    assert_eq!(response.stats.fetched_record_count, 280);
    assert_eq!(response.stats.exact_matched_count, 280);
    assert_eq!(response.stats.returned_count, 250);
    assert!(response.stats.working_set_satisfied);
    assert!(response.validate().is_ok());
}
