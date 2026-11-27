use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use insar_provider::boundary::{
    normalize_polygonal_geometry, ChinaBoundaryProvider, DataVBoundaryProvider,
};
use serde_json::json;

fn serve_one(body: serde_json::Value) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one DataV request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 2048];
        loop {
            let read = stream.read(&mut buffer).expect("request read");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(request).expect("HTTP request is UTF-8");
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("request path")
            .to_owned();
        let body = serde_json::to_string(&body).expect("response JSON");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("response write");
        path
    });
    (format!("http://{address}/"), server)
}

fn serve_rate_limited() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one DataV request");
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).expect("request read");
        stream
            .write_all(
                b"HTTP/1.1 403 Forbidden\r\nX-Tengine-Error: denied by rate limit\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .expect("response write");
    });
    (format!("http://{address}/"), server)
}

fn exact_named_feature(adcode: &str, name: &str, level: &str) -> serde_json::Value {
    json!({
        "type": "FeatureCollection",
        "features": [{
            "type": "Feature",
            "properties": {
                "adcode": adcode,
                "name": name,
                "level": level
            },
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 30.0]]]
            }
        }]
    })
}

fn exact_feature(adcode: &str) -> serde_json::Value {
    exact_named_feature(adcode, "巴东县", "district")
}

fn batch_feature(
    adcode: serde_json::Value,
    name: &str,
    geometry: serde_json::Value,
) -> serde_json::Value {
    json!({
        "type": "Feature",
        "properties": {
            "adcode": adcode,
            "name": name,
            "level": "city"
        },
        "geometry": geometry
    })
}

fn square(west: f64, south: f64) -> serde_json::Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, south],
            [west + 0.5, south],
            [west + 0.5, south + 0.5],
            [west, south]
        ]]
    })
}

#[test]
fn exact_adcode_resolver_uses_only_the_single_geometry_endpoint() {
    let (root, server) = serve_one(exact_feature("422823"));
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let result = provider
        .resolve_exact_adcode("422823")
        .expect("exact geometry resolves");

    assert_eq!(server.join().expect("server completes"), "/422823.json");
    assert_eq!(result.provider_object_id, "422823");
    assert_eq!(result.geometry_kind, "polygon");
    assert_eq!(
        normalize_polygonal_geometry(result.geometry.clone())
            .expect("result remains authoritative")
            .bbox,
        result.bbox
    );
}

#[test]
fn exact_national_adcode_uses_the_product_canonical_china_name() {
    let (root, server) = serve_one(exact_named_feature("100000", "中华人民共和国", "country"));
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let result = provider
        .resolve_exact_adcode("100000")
        .expect("exact national geometry resolves");

    assert_eq!(server.join().expect("server completes"), "/100000.json");
    assert_eq!(result.provider_object_id, "100000");
    assert_eq!(result.name, "中国");
    assert_eq!(result.display_name, "中国");
    assert_eq!(result.admin_level.as_deref(), Some("country"));
}

#[test]
fn exact_adcode_resolver_rejects_a_different_source_identity() {
    let (root, server) = serve_one(exact_feature("422824"));
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let error = provider.resolve_exact_adcode("422823").unwrap_err();

    assert_eq!(server.join().expect("server completes"), "/422823.json");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_IDENTITY_MISMATCH");
}

#[test]
fn exact_adcode_resolver_rejects_noncanonical_path_input_before_network() {
    let provider = DataVBoundaryProvider::with_test_root("http://127.0.0.1:9/")
        .expect("bounded loopback configuration");

    let error = provider.resolve_exact_adcode("422823_full").unwrap_err();

    assert_eq!(error.code, "BOUNDARY_PROVIDER_REQUEST_INVALID");
}

#[test]
fn exact_adcode_resolver_preserves_rate_limit_status_as_run_blocking_evidence() {
    let (root, server) = serve_rate_limited();
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let error = provider.resolve_exact_adcode("422823").unwrap_err();

    server.join().expect("server completes");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_RATE_LIMITED");
    assert_eq!(error.debug_context["http_status"], 403);
    assert_eq!(error.debug_context["rate_limited"], true);
}

#[test]
fn diagnostic_status_does_not_change_the_existing_china_provider_error_contract() {
    let (root, server) = serve_rate_limited();
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let error = provider
        .search("湖北省", "全部", "全部", "", 1)
        .unwrap_err();

    server.join().expect("server completes");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_HTTP_ERROR");
    assert_eq!(error.debug_context["reason"], "http_status");
    assert!(!error.debug_context.contains_key("http_status"));
}

#[test]
fn full_batch_transport_indexes_features_only_by_strict_source_adcode() {
    let body = json!({
        "type": "FeatureCollection",
        "features": [
            batch_feature(json!("420100"), "名称不参与身份匹配", square(114.0, 30.0)),
            batch_feature(json!(420200), "顺序也不参与身份匹配", square(115.0, 29.0))
        ]
    });
    let (root, server) = serve_one(body);
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let batch = provider
        .resolve_adcode_geometry_batch("420000")
        .expect("strict adcode batch resolves");

    assert_eq!(
        server.join().expect("server completes"),
        "/420000_full.json"
    );
    assert_eq!(batch.transport_adcode, "420000");
    assert_eq!(
        batch.features.keys().cloned().collect::<Vec<_>>(),
        ["420100", "420200"]
    );
    assert!(batch.invalid_features.is_empty());
    for (adcode, feature) in batch.features {
        assert_eq!(feature.provider_object_id, adcode);
        assert_eq!(
            normalize_polygonal_geometry(feature.geometry)
                .expect("batch geometry remains authoritative")
                .bbox,
            feature.bbox
        );
    }
}

#[test]
fn full_batch_transport_isolates_an_invalid_geometry_by_exact_adcode() {
    let body = json!({
        "type": "FeatureCollection",
        "features": [
            batch_feature(json!("420100"), "valid", square(114.0, 30.0)),
            batch_feature(json!("420200"), "invalid", json!({"type": "Point", "coordinates": [115.0, 30.0]}))
        ]
    });
    let (root, server) = serve_one(body);
    let provider = DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

    let batch = provider
        .resolve_adcode_geometry_batch("420000")
        .expect("one invalid feature does not erase valid batch evidence");

    assert_eq!(
        server.join().expect("server completes"),
        "/420000_full.json"
    );
    assert_eq!(batch.features.len(), 1);
    assert!(batch.features.contains_key("420100"));
    assert_eq!(batch.invalid_features.len(), 1);
    assert_eq!(
        batch.invalid_features[0].provider_object_id.as_deref(),
        Some("420200")
    );
    assert_eq!(
        batch.invalid_features[0].code,
        "BOUNDARY_GEOMETRY_UNAVAILABLE"
    );
}

#[test]
fn full_batch_transport_rejects_duplicate_or_noncanonical_feature_adcodes() {
    for features in [
        vec![
            batch_feature(json!("420100"), "first", square(114.0, 30.0)),
            batch_feature(json!("420100"), "duplicate", square(115.0, 30.0)),
        ],
        vec![batch_feature(json!("42010"), "short", square(114.0, 30.0))],
    ] {
        let (root, server) = serve_one(json!({
            "type": "FeatureCollection",
            "features": features
        }));
        let provider =
            DataVBoundaryProvider::with_test_root(&root).expect("loopback DataV provider");

        let error = provider
            .resolve_adcode_geometry_batch("420000")
            .unwrap_err();

        assert_eq!(
            server.join().expect("server completes"),
            "/420000_full.json"
        );
        assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");
    }
}

#[test]
fn full_batch_transport_rejects_a_noncanonical_transport_adcode_before_network() {
    let provider = DataVBoundaryProvider::with_test_root("http://127.0.0.1:9/")
        .expect("bounded loopback configuration");

    let error = provider
        .resolve_adcode_geometry_batch("420000_full")
        .unwrap_err();

    assert_eq!(error.code, "BOUNDARY_PROVIDER_REQUEST_INVALID");
}
