//! Source identity, attribution and response-shape policy shared by the
//! administrative boundary providers. Every request goes to a loopback fixture.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use insar_provider::boundary::{
    classify_datav_crs, wgs84_to_gcj02, DataVBoundaryProvider, DatavCrsVerdict,
    GlobalBoundaryProvider, NominatimBoundaryProvider,
};
use serde_json::{json, Value};

struct Reply {
    status: &'static str,
    content_type: &'static str,
    body: String,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: "200 OK",
            content_type: "application/json",
            body: serde_json::to_string(&value).expect("fixture JSON"),
        }
    }
}

/// Serve the replies in order, one connection each, and return the raw
/// request heads.
fn serve(replies: Vec<Reply>) -> (SocketAddr, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("bounded read");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).expect("request bytes");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            requests.push(String::from_utf8(request).expect("UTF-8 request head"));
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status,
                reply.content_type,
                reply.body.len(),
                reply.body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response bytes");
        }
        requests
    });
    (address, server)
}

fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn product_version() -> String {
    let config: Value = serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json"))
        .expect("Tauri configuration JSON");
    config["version"]
        .as_str()
        .expect("product version")
        .to_owned()
}

fn datav_polygon(adcode: &str, name: &str) -> Value {
    json!({
        "type": "FeatureCollection",
        "features": [{
            "type": "Feature",
            "properties": {"adcode": adcode, "name": name, "level": "district"},
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 30.0]]]
            }
        }]
    })
}

#[test]
fn boundary_clients_identify_scene_port_with_version_and_contact() {
    let expected = format!(
        "EOStage/{} (+https://github.com/hhanmj/EOStage)",
        product_version()
    );

    let (address, server) = serve(vec![Reply::json(json!([]))]);
    let nominatim =
        NominatimBoundaryProvider::with_test_endpoint(&format!("http://{address}/search"))
            .expect("loopback Nominatim");
    nominatim
        .discover("Kathmandu", 1)
        .expect("empty candidate list");
    let requests = server.join().expect("Nominatim fixture");
    assert_eq!(header(&requests[0], "user-agent"), Some(expected.as_str()));

    let (address, server) = serve(vec![Reply::json(datav_polygon("422823", "巴东县"))]);
    let datav = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV");
    datav
        .resolve_exact_adcode("422823")
        .expect("exact DataV unit");
    let requests = server.join().expect("DataV fixture");
    assert_eq!(header(&requests[0], "user-agent"), Some(expected.as_str()));
}

#[test]
fn each_boundary_source_carries_its_own_attribution_and_license() {
    let (address, server) = serve(vec![Reply::json(datav_polygon("422823", "巴东县"))]);
    let datav = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV");
    let feature = datav
        .resolve_exact_adcode("422823")
        .expect("exact DataV unit");
    server.join().expect("DataV fixture");
    let attribution = feature
        .attribution
        .expect("DataV results carry attribution");
    assert!(attribution.contains("DataV"), "{attribution}");
    assert!(attribution.contains("高德"), "{attribution}");
    assert!(attribution.contains("许可"), "{attribution}");

    let (address, server) = serve(vec![Reply::json(json!([{
        "osm_id": 123,
        "osm_type": "relation",
        "display_name": "Bagmati Province, Nepal",
        "name": "Bagmati Province",
        "type": "administrative",
        "address": {"country_code": "np"},
        "boundingbox": ["27.0", "28.5", "84.4", "86.6"]
    }]))]);
    let nominatim =
        NominatimBoundaryProvider::with_test_endpoint(&format!("http://{address}/search"))
            .expect("loopback Nominatim");
    let candidates = nominatim.discover("Bagmati", 1).expect("one candidate");
    server.join().expect("Nominatim fixture");
    let attribution = candidates[0]
        .attribution
        .as_deref()
        .expect("OSM attribution");
    assert!(
        attribution.contains("© OpenStreetMap contributors"),
        "{attribution}"
    );
    assert!(attribution.contains("ODbL"), "{attribution}");
}

#[test]
fn a_200_html_error_page_is_a_typed_non_data_error() {
    let html = || Reply {
        status: "200 OK",
        content_type: "text/html; charset=utf-8",
        body: "<!DOCTYPE html><html><body>Proxy login required</body></html>".to_owned(),
    };
    let disguised = || Reply {
        status: "200 OK",
        content_type: "application/json",
        body: "\r\n  <html><body>gateway error</body></html>".to_owned(),
    };

    for reply in [html(), disguised()] {
        let (address, server) = serve(vec![reply]);
        let nominatim =
            NominatimBoundaryProvider::with_test_endpoint(&format!("http://{address}/search"))
                .expect("loopback Nominatim");
        let error = nominatim
            .discover("Kathmandu", 1)
            .expect_err("HTML is not boundary data");
        server.join().expect("Nominatim fixture");
        assert_eq!(error.code, "BOUNDARY_PROVIDER_NON_DATA_RESPONSE");
    }

    for reply in [html(), disguised()] {
        let (address, server) = serve(vec![reply]);
        let datav = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
            .expect("loopback DataV");
        let error = datav
            .resolve_exact_adcode("422823")
            .expect_err("HTML is not boundary data");
        server.join().expect("DataV fixture");
        assert_eq!(error.code, "BOUNDARY_PROVIDER_NON_DATA_RESPONSE");
        assert!(
            error.user_message.contains("代理"),
            "{}",
            error.user_message
        );
    }
}

fn square(west: f64, south: f64, size: f64) -> Vec<[f64; 2]> {
    vec![
        [west, south],
        [west + size, south],
        [west + size, south + size],
        [west, south + size],
        [west, south],
    ]
}

fn polygon(ring: &[[f64; 2]]) -> Value {
    json!({"type": "Polygon", "coordinates": [ring]})
}

#[test]
fn datav_crs_classifier_tells_gcj02_from_wgs84_against_a_wgs84_reference() {
    // A Hong Kong sized WGS84 reference outline.
    let reference = square(114.0, 22.2, 0.3);
    let shifted_to_gcj = reference
        .iter()
        .map(|[lng, lat]| {
            let (lng, lat) = wgs84_to_gcj02(*lng, *lat);
            [lng, lat]
        })
        .collect::<Vec<_>>();
    let unrelated = reference
        .iter()
        .map(|[lng, lat]| [lng - 0.05, lat + 0.04])
        .collect::<Vec<_>>();

    assert_eq!(
        classify_datav_crs(&polygon(&shifted_to_gcj), &polygon(&reference)),
        DatavCrsVerdict::Gcj02
    );
    assert_eq!(
        classify_datav_crs(&polygon(&reference), &polygon(&reference)),
        DatavCrsVerdict::Wgs84
    );
    assert_eq!(
        classify_datav_crs(&polygon(&unrelated), &polygon(&reference)),
        DatavCrsVerdict::Inconclusive
    );
}
