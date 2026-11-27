//! geoBoundaries gbOpen country/level browsing. All traffic goes to a loopback
//! fixture; the ADM1 metadata object is a recorded public API response.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use geo::{LineString, MultiPolygon, Polygon};
use insar_provider::boundary::GlobalBoundaryProvider;
use insar_provider::geoboundaries::{
    disputed_guard_region, GeoBoundariesProvider, GeoBoundariesRelease, GeoBoundaryCatalog,
};
use serde_json::{json, Value};

const RECORDED_NPL_ADM1: &str =
    include_str!("fixtures/geoboundaries/api-current-gbopen-npl-adm1.json");
/// Real gbOpen 9469f09 units exported in L4 round 4, coordinates unchanged.
const R4_REAL_UNITS: &str = include_str!("fixtures/geoboundaries/r4-real-units.geojson");

/// One real unit from the L4 r4 export, as the pinned layer serves it.
fn real_unit(iso: &str, level: &str, name: &str) -> Value {
    let collection: Value = serde_json::from_str(R4_REAL_UNITS).expect("r4 real units");
    collection["features"]
        .as_array()
        .expect("features")
        .iter()
        .find(|feature| {
            let properties = &feature["properties"];
            properties["shapeGroup"] == iso
                && properties["shapeType"] == level
                && properties["shapeName"] == name
        })
        .cloned()
        .unwrap_or_else(|| panic!("{iso} {level} {name} is in the r4 export"))
}

/// Serve one layer (metadata + geometry) for `requests` requests.
fn serve_layer(
    iso: &str,
    level: &str,
    features: Vec<Value>,
    requests: usize,
) -> (SocketAddr, JoinHandle<Vec<String>>) {
    serve(
        BTreeMap::from([
            (
                format!("/api/current/gbOpen/{iso}/{level}/"),
                json_reply(&metadata(iso, level, "9469f09")),
            ),
            (
                layer_path("9469f09", iso, level),
                json_reply(&json!({"type": "FeatureCollection", "features": features})),
            ),
        ]),
        requests,
    )
}

struct Reply {
    content_type: &'static str,
    body: String,
}

fn json_reply(value: &Value) -> Reply {
    Reply {
        content_type: "application/json",
        body: serde_json::to_string(value).expect("fixture JSON"),
    }
}

/// Answer up to `count` requests by exact path and return the requested
/// paths. Stops waiting after five seconds so a provider that never connects
/// fails its assertion instead of hanging the test binary on `join`.
fn serve(routes: BTreeMap<String, Reply>, count: usize) -> (SocketAddr, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener
        .set_nonblocking(true)
        .expect("nonblocking loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut paths = Vec::new();
        while paths.len() < count {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            };
            stream
                .set_nonblocking(false)
                .expect("blocking fixture stream");
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
            let head = String::from_utf8(request).expect("UTF-8 request head");
            let path = head
                .split_whitespace()
                .nth(1)
                .expect("request path")
                .to_owned();
            let response = match routes.get(&path) {
                Some(reply) => format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.content_type,
                    reply.body.len(),
                    reply.body
                ),
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned(),
            };
            // A client that refuses an oversized body closes early.
            let _ = stream.write_all(response.as_bytes());
            paths.push(path);
        }
        paths
    });
    (address, server)
}

fn provider(address: SocketAddr, release: GeoBoundariesRelease) -> GeoBoundariesProvider {
    GeoBoundariesProvider::with_test_roots(
        &format!("http://{address}/api/current/"),
        &format!("http://{address}/media/wmgeolab/geoBoundaries/"),
        release,
    )
    .expect("loopback geoBoundaries provider")
}

fn unreachable_provider() -> GeoBoundariesProvider {
    GeoBoundariesProvider::with_test_roots(
        "http://127.0.0.1:9/api/current/",
        "http://127.0.0.1:9/media/wmgeolab/geoBoundaries/",
        GeoBoundariesRelease::pinned(),
    )
    .expect("loopback geoBoundaries provider")
}

fn metadata(iso: &str, level: &str, data_ref: &str) -> Value {
    let mut value: Value = serde_json::from_str(RECORDED_NPL_ADM1).expect("recorded metadata");
    let object = value.as_object_mut().expect("metadata object");
    object.insert("boundaryISO".to_owned(), json!(iso));
    object.insert("boundaryType".to_owned(), json!(level));
    object.insert(
        "gjDownloadURL".to_owned(),
        json!(format!(
            "https://github.com/wmgeolab/geoBoundaries/raw/{data_ref}/releaseData/gbOpen/{iso}/{level}/geoBoundaries-{iso}-{level}.geojson"
        )),
    );
    object.insert(
        "simplifiedGeometryGeoJSON".to_owned(),
        json!(format!(
            "https://github.com/wmgeolab/geoBoundaries/raw/{data_ref}/releaseData/gbOpen/{iso}/{level}/geoBoundaries-{iso}-{level}_simplified.geojson"
        )),
    );
    value
}

fn square(west: f64, south: f64, size: f64) -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, south], [west + size, south], [west + size, south + size],
            [west, south + size], [west, south]
        ]]
    })
}

fn unit(iso: &str, level: &str, shape_id: &str, name: &str, geometry: Value) -> Value {
    json!({
        "type": "Feature",
        "properties": {
            "shapeName": name,
            "shapeISO": "",
            "shapeID": shape_id,
            "shapeGroup": iso,
            "shapeType": level
        },
        "geometry": geometry
    })
}

fn layer_path(data_ref: &str, iso: &str, level: &str) -> String {
    format!(
        "/media/wmgeolab/geoBoundaries/{data_ref}/releaseData/gbOpen/{iso}/{level}/geoBoundaries-{iso}-{level}_simplified.geojson"
    )
}

#[test]
fn recorded_api_object_uses_the_pinned_release_commit() {
    let recorded: Value = serde_json::from_str(RECORDED_NPL_ADM1).expect("recorded metadata");
    let release = GeoBoundariesRelease::pinned();
    assert_eq!(release.release_type, "gbOpen");
    assert_eq!(release.version, "6.0.0");
    assert_eq!(release.data_ref, "9469f09");
    assert!(recorded["gjDownloadURL"]
        .as_str()
        .unwrap()
        .contains(&format!("/raw/{}/", release.data_ref)));
}

#[test]
fn country_list_never_offers_china_scope_entries() {
    let countries = json!([
        {"boundaryISO": "NPL", "boundaryName": "Nepal", "boundaryType": "ADM0"},
        {"boundaryISO": "CHN", "boundaryName": "China", "boundaryType": "ADM0"},
        {"boundaryISO": "TWN", "boundaryName": "Taiwan", "boundaryType": "ADM0"},
        {"boundaryISO": "HKG", "boundaryName": "Hong Kong", "boundaryType": "ADM0"},
        {"boundaryISO": "MAC", "boundaryName": "Macao", "boundaryType": "ADM0"},
        {"boundaryISO": "FRA", "boundaryName": "France", "boundaryType": "ADM0"}
    ]);
    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/ALL/ADM0/".to_owned(),
            json_reply(&countries),
        )]),
        1,
    );
    let list = provider(address, GeoBoundariesRelease::pinned())
        .countries()
        .expect("country list");
    server.join().expect("fixture server");
    let codes = list
        .iter()
        .map(|item| item.iso3.as_str())
        .collect::<Vec<_>>();
    assert_eq!(codes, vec!["FRA", "NPL"]);
    assert_eq!(list[1].name, "Nepal");
}

#[test]
fn china_scope_codes_are_refused_before_any_request() {
    let provider = unreachable_provider();
    for iso in ["CHN", "twn", "HKG", "mac"] {
        let error = provider
            .levels(iso)
            .expect_err("China scope never routes here");
        assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE", "{iso}");
        assert_eq!(error.debug_context["fallback"], "china_selector", "{iso}");
        let error = provider
            .layer(iso, "ADM1")
            .expect_err("China scope never routes here");
        assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE", "{iso}");
    }
    let error = provider
        .resolve("gb:TWN:ADM1:TWN-ADM1-1B1")
        .expect_err("China scope identity is refused");
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE");
    let error = provider
        .discover_china("台北", 5)
        .expect_err("no China discovery");
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE");
    let error = provider
        .search_taiwan("台北", 5)
        .expect_err("no Taiwan search");
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_CHINA_SCOPE");
}

#[test]
fn levels_follow_the_recorded_api_shape() {
    let mut adm0 = metadata("NPL", "ADM0", "9469f09");
    adm0["boundaryCanonical"] = json!("nan");
    adm0["admUnitCount"] = json!("1");
    let adm1 = metadata("NPL", "ADM1", "9469f09");
    let mut adm2 = metadata("NPL", "ADM2", "9469f09");
    adm2["boundaryCanonical"] = json!("Districts");
    adm2["admUnitCount"] = json!("77");
    adm2["boundaryLicense"] = json!("Open Data Commons Open Database License 1.0");
    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/NPL/ALL/".to_owned(),
            json_reply(&json!([adm2, adm0, adm1])),
        )]),
        1,
    );
    let levels = provider(address, GeoBoundariesRelease::pinned())
        .levels("npl")
        .expect("levels");
    server.join().expect("fixture server");

    assert_eq!(
        levels
            .iter()
            .map(|level| level.level.as_str())
            .collect::<Vec<_>>(),
        vec!["ADM0", "ADM1", "ADM2"]
    );
    assert_eq!(levels[0].canonical_name, None, "pandas nan is missing");
    let adm1 = &levels[1];
    assert_eq!(adm1.iso3, "NPL");
    assert_eq!(adm1.canonical_name.as_deref(), Some("Provinces"));
    assert_eq!(adm1.unit_count, Some(7));
    assert_eq!(
        adm1.license,
        "Creative Commons Attribution 3.0 Intergovernmental Organisations (CC BY 3.0 IGO)"
    );
    assert_eq!(
        adm1.source.as_deref(),
        Some("Survey Department of Nepal, OCHA FISS")
    );
    assert!(adm1.attribution.contains("geoBoundaries gbOpen 6.0.0"));
    assert!(adm1.attribution.contains("CC BY 3.0 IGO"));
    assert!(levels[2].attribution.contains("Open Database License"));
}

#[test]
fn layer_geometry_comes_from_the_pinned_release_and_binds_shape_identity() {
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [
            unit("NPL", "ADM1", "NPL-ADM1-1B1", "Koshi", square(86.0, 26.5, 1.0)),
            unit("NPL", "ADM1", "NPL-ADM1-1B3", "Bagmati", square(85.0, 27.0, 1.0)),
            unit("NPL", "ADM2", "NPL-ADM2-9B9", "Wrong level", square(84.0, 27.0, 0.5)),
            unit("NPL", "ADM1", "NPL-ADM1-1B4", "Point", json!({"type": "Point", "coordinates": [84.0, 28.0]}))
        ]
    });
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "9469f09")),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        2,
    );
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "adm1")
        .expect("pinned layer");
    let paths = server.join().expect("fixture server");
    assert_eq!(paths[1], layer_path("9469f09", "NPL", "ADM1"));
    assert_eq!(layer.release_drift, None);
    assert_eq!(layer.level, "ADM1");
    assert_eq!(layer.invalid_units, 2);
    assert_eq!(layer.withheld_disputed, 0);
    let names = layer
        .units
        .iter()
        .map(|unit| unit.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Bagmati", "Koshi"]);
    assert_eq!(
        layer.units[0].provider_object_id,
        "gb:NPL:ADM1:NPL-ADM1-1B3"
    );
    assert_eq!(layer.units[0].geometry_kind, "polygon");
    assert_eq!(layer.units[0].bbox, [85.0, 27.0, 86.0, 28.0]);
    assert!(layer
        .attribution
        .contains("geoBoundaries gbOpen 6.0.0 (9469f09)"));
    assert!(layer.attribution.contains("CC BY 3.0 IGO"));
}

#[test]
fn configured_release_changes_the_geometry_path() {
    let release = GeoBoundariesRelease::new("6.1.0", "0123abc").expect("valid release");
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [unit("NPL", "ADM1", "NPL-ADM1-1B1", "Koshi", square(86.0, 26.5, 1.0))]
    });
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "0123abc")),
            ),
            (
                layer_path("0123abc", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        2,
    );
    let layer = provider(address, release)
        .layer("NPL", "ADM1")
        .expect("configured layer");
    server.join().expect("fixture server");
    assert_eq!(layer.release_drift, None);
    assert!(layer.attribution.contains("6.1.0 (0123abc)"));
    assert!(GeoBoundariesRelease::new("6.1.0", "../main").is_err());
}

#[test]
fn units_touching_disputed_guard_regions_are_withheld() {
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [
            unit("IND", "ADM1", "IND-ADM1-AR", "Arunachal Pradesh", square(93.0, 27.6, 1.0)),
            unit("IND", "ADM1", "IND-ADM1-KL", "Kerala", square(76.0, 9.0, 1.0))
        ]
    });
    let routes = || {
        BTreeMap::from([
            (
                "/api/current/gbOpen/IND/ADM1/".to_owned(),
                json_reply(&metadata("IND", "ADM1", "9469f09")),
            ),
            (
                layer_path("9469f09", "IND", "ADM1"),
                json_reply(&layer_body),
            ),
        ])
    };
    let (address, server) = serve(routes(), 2);
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("IND", "ADM1")
        .expect("layer");
    server.join().expect("fixture server");
    assert_eq!(layer.withheld_disputed, 1);
    assert_eq!(
        layer
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Kerala"]
    );

    let (address, server) = serve(routes(), 2);
    let error = provider(address, GeoBoundariesRelease::pinned())
        .resolve("gb:IND:ADM1:IND-ADM1-AR")
        .expect_err("disputed unit never binds");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_DISPUTED");
    assert_eq!(error.debug_context["fallback"], "china_selector");
    assert!(
        error.user_message.contains("该区域请使用中国行政区选择器"),
        "{}",
        error.user_message
    );
}

/// A ~200 m square around a named public reference point (lon, lat).
fn place(lon: f64, lat: f64) -> MultiPolygon<f64> {
    let half = 0.001;
    MultiPolygon::new(vec![Polygon::new(
        LineString::from(vec![
            (lon - half, lat - half),
            (lon + half, lat - half),
            (lon + half, lat + half),
            (lon - half, lat + half),
            (lon - half, lat - half),
        ]),
        Vec::new(),
    )])
}

#[test]
fn guard_rings_cover_claimed_places_and_spare_their_neighbours() {
    // Places inside the Chinese claim: units containing them are withheld.
    let claimed = [
        ("Itanagar", 93.61, 27.08),
        ("Tawang", 91.86, 27.59),
        ("Pasighat", 95.33, 28.07),
        ("Longding", 95.34, 26.92),
        ("Khonsa (Tirap)", 95.50, 27.01),
        ("Changlang", 95.73, 27.13),
        ("Aksai Chin", 79.30, 35.20),
        ("Shaksgam valley", 76.60, 36.20),
        ("Barahoti", 79.97, 30.82),
        ("Shipki La west", 78.73, 31.82),
        ("Kaurik", 78.58, 32.08),
        ("Nelang", 79.00, 31.08),
        ("Doklam", 89.00, 27.32),
        // Samtse's north tip north of the Gipmochi-Jampheri ridge.
        ("Samtse north tip", 88.96, 27.285),
        ("Diaoyu Dao", 123.47, 25.74),
        ("Yongxing Dao", 112.33, 16.83),
        ("Huangyan Dao", 117.76, 15.15),
        ("Taiping Dao", 114.37, 10.38),
    ];
    for (name, lon, lat) in claimed {
        assert!(
            disputed_guard_region(&place(lon, lat)).is_some(),
            "{name} must be withheld"
        );
    }
    // Neighbours that only share a border with a claimed area stay listed.
    let neighbours = [
        ("Dibrugarh", 94.91, 27.47),
        ("Tinsukia", 95.36, 27.49),
        ("North Lakhimpur", 94.10, 27.24),
        ("Sadiya", 95.67, 27.83),
        ("Margherita", 95.67, 27.29),
        ("Tezpur", 92.80, 26.63),
        ("Mon (Nagaland)", 95.00, 26.73),
        ("Concordia", 76.51, 35.74),
        ("K2 summit", 76.51, 35.88),
        ("Skardu", 75.63, 35.30),
        ("Leh", 77.58, 34.16),
        ("Kaza", 78.07, 32.23),
        ("Joshimath", 79.56, 30.56),
        ("Gangtok", 88.61, 27.33),
        ("Sakteng", 91.96, 27.36),
        ("Thimphu", 89.64, 27.47),
        ("Paro", 89.41, 27.43),
        ("Haa", 89.28, 27.39),
        ("Punakha", 89.86, 27.58),
        ("Bumthang", 90.73, 27.55),
        ("Trashigang", 91.55, 27.33),
        ("Trashi Yangtse", 91.50, 27.61),
        ("Samdrup Jongkhar", 91.50, 26.80),
        ("Gelephu", 90.48, 26.87),
        ("Phuentsholing", 89.39, 26.85),
        ("Kathmandu", 85.32, 27.71),
        ("Putao", 97.40, 27.33),
        ("Ishigaki", 124.16, 24.34),
        ("Da Nang", 108.20, 16.05),
        ("Puerto Princesa", 118.74, 9.74),
        ("Kota Kinabalu", 116.07, 5.98),
        // Representative points of the real gbOpen 9469f09 units (L4 r4).
        ("Tinsukia district", 95.5603, 27.598),
        ("Dhemaji district", 94.5545, 27.544),
        ("Lakhimpur district", 94.1648, 27.151),
        ("Dibrugarh district", 95.0452, 27.3736),
        ("Charaideo district", 95.0542, 27.0589),
        ("Mon district", 95.0097, 26.6513),
        ("Hunza district", 75.2606, 36.5129),
        ("Gilgit district", 74.4765, 35.9874),
        ("Gilgit-Baltistan", 74.9476, 35.7809),
    ];
    for (name, lon, lat) in neighbours {
        assert_eq!(
            disputed_guard_region(&place(lon, lat)),
            None,
            "{name} must stay listed"
        );
    }
}

#[test]
fn resolve_returns_the_exact_unit_with_attribution() {
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [
            unit("NPL", "ADM1", "NPL-ADM1-1B1", "Koshi", square(86.0, 26.5, 1.0)),
            unit("NPL", "ADM1", "NPL-ADM1-1B3", "Bagmati", square(85.0, 27.0, 1.0))
        ]
    });
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "9469f09")),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        2,
    );
    let feature = provider(address, GeoBoundariesRelease::pinned())
        .resolve("gb:NPL:ADM1:NPL-ADM1-1B1")
        .expect("exact unit");
    server.join().expect("fixture server");
    assert_eq!(feature.provider_object_id, "gb:NPL:ADM1:NPL-ADM1-1B1");
    assert_eq!(feature.name, "Koshi");
    // country_code is ISO 3166-1 alpha-2 elsewhere; geoBoundaries only gives
    // alpha-3, which stays in the identity and display name instead.
    assert_eq!(feature.country_code, None);
    assert!(feature.display_name.contains("NPL"));
    assert_eq!(feature.admin_level.as_deref(), Some("ADM1"));
    assert_eq!(feature.geometry_kind, "polygon");
    assert!(feature
        .attribution
        .as_deref()
        .is_some_and(|text| text.contains("geoBoundaries") && text.contains("CC BY 3.0 IGO")));

    let provider = unreachable_provider();
    for malformed in [
        "NPL-ADM1-1B1",
        "gb:NPL:ADM9:x",
        "gb:NP:ADM1:x",
        "gb:NPL:ADM1:",
    ] {
        let error = provider.resolve(malformed).expect_err("malformed identity");
        assert_eq!(
            error.code, "BOUNDARY_PROVIDER_IDENTITY_INVALID",
            "{malformed}"
        );
    }
}

#[test]
fn html_error_page_from_the_api_is_a_typed_non_data_error() {
    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/ALL/ADM0/".to_owned(),
            Reply {
                content_type: "text/html",
                body: "<html><body>blocked</body></html>".to_owned(),
            },
        )]),
        1,
    );
    let error = provider(address, GeoBoundariesRelease::pinned())
        .countries()
        .expect_err("HTML is not data");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_NON_DATA_RESPONSE");
}

#[test]
fn a_layer_or_list_without_usable_entries_is_invalid_data() {
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [
            unit("NPL", "ADM2", "NPL-ADM2-9B9", "Wrong level", square(84.0, 27.0, 0.5)),
            unit("NPL", "ADM1", "NPL-ADM1-1B4", "Point", json!({"type": "Point", "coordinates": [84.0, 28.0]}))
        ]
    });
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "9469f09")),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        2,
    );
    let error = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "ADM1")
        .expect_err("a layer of invalid features is not an empty country");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");

    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/ALL/ADM0/".to_owned(),
            json_reply(&json!([
                {"boundaryISO": "CHN", "boundaryName": "China", "boundaryType": "ADM0"},
                {"boundaryISO": "TWN", "boundaryName": "Taiwan", "boundaryType": "ADM0"}
            ])),
        )]),
        1,
    );
    let error = provider(address, GeoBoundariesRelease::pinned())
        .countries()
        .expect_err("no usable country");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");

    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/NPL/ALL/".to_owned(),
            json_reply(&json!([])),
        )]),
        1,
    );
    let error = provider(address, GeoBoundariesRelease::pinned())
        .levels("NPL")
        .expect_err("no usable level");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_INVALID");
}

fn metadata_file_path(data_ref: &str, iso: &str, level: &str) -> String {
    format!(
        "/media/wmgeolab/geoBoundaries/{data_ref}/releaseData/gbOpen/{iso}/{level}/geoBoundaries-{iso}-{level}-metaData.json"
    )
}

#[test]
fn the_api_wildcard_is_not_a_country_code() {
    let provider = unreachable_provider();
    for wildcard in ["ALL", "all"] {
        let error = provider.levels(wildcard).expect_err("ALL is not a country");
        assert_eq!(
            error.code, "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "{wildcard}"
        );
        let error = provider
            .layer(wildcard, "ADM1")
            .expect_err("ALL is not a country");
        assert_eq!(
            error.code, "BOUNDARY_PROVIDER_REQUEST_INVALID",
            "{wildcard}"
        );
    }
}

#[test]
fn attribution_follows_the_pinned_release_when_the_api_has_moved_on() {
    let layer_body = json!({
        "type": "FeatureCollection",
        "features": [unit("NPL", "ADM1", "NPL-ADM1-1B1", "Koshi", square(86.0, 26.5, 1.0))]
    });
    let mut pinned = metadata("NPL", "ADM1", "9469f09");
    pinned["boundaryLicense"] = json!("Pinned Licence 1.0");
    pinned["boundarySource"] = json!("Pinned Survey");
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "abcdef1")),
            ),
            (
                metadata_file_path("9469f09", "NPL", "ADM1"),
                json_reply(&pinned),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        3,
    );
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "ADM1")
        .expect("pinned layer");
    let paths = server.join().expect("fixture server");
    assert!(paths.contains(&layer_path("9469f09", "NPL", "ADM1")));
    assert_eq!(layer.release_drift.as_deref(), Some("abcdef1"));
    assert_eq!(layer.license, "Pinned Licence 1.0");
    assert!(
        layer.attribution.contains("Pinned Survey"),
        "{}",
        layer.attribution
    );
    assert!(
        !layer.attribution.contains("CC BY 3.0 IGO"),
        "{}",
        layer.attribution
    );

    // Without readable pinned metadata nothing is borrowed from `current`.
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "abcdef1")),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                json_reply(&layer_body),
            ),
        ]),
        3,
    );
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "ADM1")
        .expect("pinned layer");
    server.join().expect("fixture server");
    assert!(layer
        .attribution
        .contains("geoBoundaries gbOpen 6.0.0 (9469f09)"));
    assert!(
        !layer.attribution.contains("CC BY 3.0 IGO"),
        "{}",
        layer.attribution
    );
    assert!(
        !layer.license.contains("CC BY 3.0 IGO"),
        "{}",
        layer.license
    );

    // A listed level whose links point at another build borrows no licence.
    let (address, server) = serve(
        BTreeMap::from([(
            "/api/current/gbOpen/NPL/ALL/".to_owned(),
            json_reply(&json!([metadata("NPL", "ADM1", "abcdef1")])),
        )]),
        1,
    );
    let levels = provider(address, GeoBoundariesRelease::pinned())
        .levels("NPL")
        .expect("levels");
    server.join().expect("fixture server");
    assert_eq!(levels[0].level, "ADM1");
    assert!(!levels[0].attribution.contains("CC BY 3.0 IGO"));
    assert!(levels[0].attribution.contains("6.0.0"));
}

#[test]
fn a_layer_over_sixteen_megabytes_is_refused() {
    let feature = unit(
        "NPL",
        "ADM1",
        "NPL-ADM1-1B1",
        "Koshi",
        square(86.0, 26.5, 1.0),
    );
    let body = format!(
        "{{\"type\":\"FeatureCollection\",\"features\":[{}]}}{}",
        serde_json::to_string(&feature).expect("fixture JSON"),
        " ".repeat(16 * 1024 * 1024)
    );
    let (address, server) = serve(
        BTreeMap::from([
            (
                "/api/current/gbOpen/NPL/ADM1/".to_owned(),
                json_reply(&metadata("NPL", "ADM1", "9469f09")),
            ),
            (
                layer_path("9469f09", "NPL", "ADM1"),
                Reply {
                    content_type: "application/json",
                    body,
                },
            ),
        ]),
        2,
    );
    let error = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "ADM1")
        .expect_err("a layer over the bound is not cached data");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_PROVIDER_RESPONSE_TOO_LARGE");
}

#[test]
fn large_real_units_clear_of_every_ring_are_listed_and_resolve() {
    // L4 r4: these were withheld although no ring touches them, because the
    // guard read their geometry through the ASF footprint vertex bounds.
    for (iso, level, name, disputed_neighbour) in [
        ("IND", "ADM1", "Mahārāshtra", Some("Arunāchal Pradesh")),
        ("MMR", "ADM1", "Kachin", None),
        ("JPN", "ADM1", "Hokkaido", None),
        ("PHL", "ADM1", "Central Luzon", None),
    ] {
        let mut features = vec![real_unit(iso, level, name)];
        features.extend(disputed_neighbour.map(|other| real_unit(iso, level, other)));
        let (address, server) = serve_layer(iso, level, features.clone(), 2);
        let layer = provider(address, GeoBoundariesRelease::pinned())
            .layer(iso, level)
            .expect("real layer");
        server.join().expect("fixture server");
        assert_eq!(
            layer
                .units
                .iter()
                .map(|unit| unit.name.as_str())
                .collect::<Vec<_>>(),
            vec![name],
            "{iso} {level}"
        );
        assert!(layer.unavailable_units.is_empty(), "{name}");
        assert_eq!(
            layer.withheld_disputed,
            usize::from(disputed_neighbour.is_some()),
            "{name}"
        );

        let id = layer.units[0].provider_object_id.clone();
        let (address, server) = serve_layer(iso, level, features, 2);
        let feature = provider(address, GeoBoundariesRelease::pinned())
            .resolve(&id)
            .expect("a listed real unit binds");
        server.join().expect("fixture server");
        assert_eq!(feature.name, name);
    }
}

/// A square whose second vertex is repeated: polygonal and clear to read,
/// but not a valid authoritative polygon.
fn repeated_vertex_square(west: f64, south: f64) -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, south], [west + 0.5, south], [west + 0.5, south],
            [west + 0.5, south + 0.5], [west, south + 0.5], [west, south]
        ]]
    })
}

#[test]
fn unverifiable_geometry_is_listed_as_unavailable_not_withheld() {
    let features = vec![
        unit(
            "NPL",
            "ADM1",
            "NPL-ADM1-1B1",
            "Koshi",
            square(86.0, 26.5, 1.0),
        ),
        unit(
            "NPL",
            "ADM1",
            "NPL-ADM1-1B9",
            "Unverified",
            repeated_vertex_square(84.0, 28.0),
        ),
        // Unverified coordinates inside a guard ring are still withheld.
        unit(
            "NPL",
            "ADM1",
            "NPL-ADM1-1B8",
            "Unverified guarded",
            repeated_vertex_square(93.0, 27.6),
        ),
    ];
    let (address, server) = serve_layer("NPL", "ADM1", features.clone(), 2);
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("NPL", "ADM1")
        .expect("layer");
    server.join().expect("fixture server");
    assert_eq!(
        layer
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Koshi"]
    );
    assert_eq!(layer.unavailable_units.len(), 1);
    let unavailable = &layer.unavailable_units[0];
    assert_eq!(unavailable.name, "Unverified");
    assert_eq!(unavailable.provider_object_id, "gb:NPL:ADM1:NPL-ADM1-1B9");
    assert_eq!(unavailable.bbox, Some([84.0, 28.0, 84.5, 28.5]));
    assert_eq!(layer.withheld_disputed, 1);
    assert_eq!(
        layer.withheld_object_ids,
        vec!["gb:NPL:ADM1:NPL-ADM1-1B8".to_owned()]
    );
    assert_eq!(layer.invalid_units, 0);

    let (address, server) = serve_layer("NPL", "ADM1", features, 2);
    let error = provider(address, GeoBoundariesRelease::pinned())
        .resolve("gb:NPL:ADM1:NPL-ADM1-1B9")
        .expect_err("an unavailable unit never binds");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_GEOMETRY_UNAVAILABLE");
    assert_eq!(error.debug_context["reason"], "geometry_unverifiable");
}

fn real_polygons(iso: &str, level: &str, name: &str) -> MultiPolygon<f64> {
    let geometry = real_unit(iso, level, name)["geometry"].clone();
    let geometry = geojson::Geometry::from_json_value(geometry).expect("GeoJSON geometry");
    match geo::Geometry::<f64>::try_from(geometry.value).expect("geo geometry") {
        geo::Geometry::Polygon(polygon) => MultiPolygon::new(vec![polygon]),
        geo::Geometry::MultiPolygon(polygons) => polygons,
        other => panic!("{name} is polygonal, got {other:?}"),
    }
}

#[test]
fn guard_rings_spare_real_neighbours_and_keep_claimed_units() {
    // L4 r4: these real units only border a guarded area but touched the
    // guard-east-01, guard-east-02 and guard-west-02 rings.
    for (iso, level, name) in [
        ("IND", "ADM1", "Assam"),
        ("IND", "ADM1", "Nāgāland"),
        ("IND", "ADM2", "Tinsukia"),
        ("IND", "ADM2", "Dhemaji"),
        ("IND", "ADM2", "Lakhimpur"),
        ("IND", "ADM2", "Dibrugarh"),
        ("IND", "ADM2", "Charaideo"),
        ("IND", "ADM2", "Mon"),
        ("PAK", "ADM1", "Gilgit-Baltistan"),
        ("PAK", "ADM2", "Hunza"),
        ("PAK", "ADM2", "Gilgit"),
    ] {
        assert_eq!(
            disputed_guard_region(&real_polygons(iso, level, name)),
            None,
            "{iso} {level} {name} must stay listed"
        );
    }
    for (iso, level, name) in [
        ("IND", "ADM1", "Arunāchal Pradesh"),
        ("IND", "ADM2", "Namsai"),
        ("IND", "ADM2", "Tirap"),
        ("IND", "ADM2", "Longding"),
        ("IND", "ADM2", "Changlang"),
        ("IND", "ADM2", "Papum Pare"),
        ("BTN", "ADM1", "Haa"),
        // Its north tip lies in the guard-southwest-01 ring.
        ("BTN", "ADM1", "Samtse"),
    ] {
        assert!(
            disputed_guard_region(&real_polygons(iso, level, name)).is_some(),
            "{iso} {level} {name} must be withheld"
        );
    }
}

#[test]
fn okinawa_and_ishigaki_are_withheld_by_identity() {
    // gbOpen draws neither unit over Diaoyu Dao, so no guard ring catches
    // them (L4 r4 E-geo-ring-diagnosis.json).
    let okinawa = real_unit("JPN", "ADM1", "Okinawa Prefecture");
    let features = vec![okinawa.clone(), real_unit("JPN", "ADM1", "Hokkaido")];
    let (address, server) = serve_layer("JPN", "ADM1", features.clone(), 2);
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("JPN", "ADM1")
        .expect("JPN ADM1");
    server.join().expect("fixture server");
    assert_eq!(
        layer
            .units
            .iter()
            .map(|unit| unit.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Hokkaido"]
    );
    assert_eq!(
        layer.withheld_object_ids,
        vec!["gb:JPN:ADM1:47310658B31569081891025".to_owned()]
    );

    let (address, server) = serve_layer("JPN", "ADM1", features, 2);
    let error = provider(address, GeoBoundariesRelease::pinned())
        .resolve("gb:JPN:ADM1:47310658B31569081891025")
        .expect_err("Okinawa never binds");
    server.join().expect("fixture server");
    assert_eq!(error.code, "BOUNDARY_ROUTE_REFUSED_DISPUTED");
    assert_eq!(error.debug_context["fallback"], "china_selector");

    // A later release that renumbers the shapeID is still matched by name.
    let mut renumbered = okinawa;
    renumbered["properties"]["shapeID"] = json!("JPN-ADM1-RENUMBERED");
    let (address, server) = serve_layer("JPN", "ADM1", vec![renumbered], 2);
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("JPN", "ADM1")
        .expect("JPN ADM1");
    server.join().expect("fixture server");
    assert!(layer.units.is_empty());
    assert_eq!(layer.withheld_disputed, 1);

    let (address, server) =
        serve_layer("JPN", "ADM2", vec![real_unit("JPN", "ADM2", "Ishigaki")], 2);
    let layer = provider(address, GeoBoundariesRelease::pinned())
        .layer("JPN", "ADM2")
        .expect("JPN ADM2");
    server.join().expect("fixture server");
    assert!(layer.units.is_empty());
    assert_eq!(
        layer.withheld_object_ids,
        vec!["gb:JPN:ADM2:22064153B46764438077406".to_owned()]
    );
}
