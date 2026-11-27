use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use insar_provider::boundary::{
    normalize_polygonal_geometry, ChinaBoundaryProvider, DataVBoundaryProvider,
    GlobalBoundaryProvider, NominatimBoundaryProvider, NominatimQuery, NOMINATIM_SEARCH_URL,
};
use serde_json::json;

const LOOPBACK_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const LOOPBACK_REQUEST_MAX_BYTES: usize = 64 * 1024;

fn read_bounded_http_request(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_nonblocking(false)
        .expect("accepted stream becomes blocking");
    stream
        .set_read_timeout(Some(LOOPBACK_REQUEST_READ_TIMEOUT))
        .expect("accepted stream gets a bounded read timeout");

    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    loop {
        let read = stream
            .read(&mut buffer)
            .expect("bounded loopback request bytes");
        if read == 0 {
            break;
        }
        assert!(
            request.len() + read <= LOOPBACK_REQUEST_MAX_BYTES,
            "loopback HTTP request exceeds the bounded test-harness limit"
        );
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    assert!(
        request.windows(4).any(|window| window == b"\r\n\r\n"),
        "loopback HTTP request ended before complete headers"
    );
    request
}

#[test]
fn datav_selector_exposes_the_complete_province_level_authority_without_network() {
    let provider = DataVBoundaryProvider::with_test_root("http://127.0.0.1:9/")
        .expect("bounded loopback configuration");
    let options = provider.options("", "").expect("static province authority");
    let unique = options
        .provinces
        .iter()
        .map(|option| option.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        options.provinces.len(),
        35,
        "全国 plus 34 province-level divisions"
    );
    assert_eq!(unique.len(), options.provinces.len());
    for required in [
        "黑龙江省",
        "宁夏回族自治区",
        "新疆维吾尔自治区",
        "西藏自治区",
        "台湾省",
        "香港特别行政区",
        "澳门特别行政区",
    ] {
        let option = options
            .provinces
            .iter()
            .find(|option| option.name == required)
            .expect("province option");
        assert!(option.provider_object_id.as_deref().is_some_and(
            |adcode| adcode.len() == 6 && adcode.bytes().all(|byte| byte.is_ascii_digit())
        ));
    }
    assert_eq!(
        options.provinces[0].provider_object_id.as_deref(),
        Some("100000")
    );
}

#[test]
fn datav_national_aliases_share_one_country_selection() {
    let provider = DataVBoundaryProvider::with_test_root("http://127.0.0.1:9/")
        .expect("bounded loopback configuration");
    for alias in ["", "全国", "全部", "不限", "中国", "China"] {
        let options = provider
            .options(alias, "全部")
            .expect("national aliases use the static country selector");
        assert_eq!(options.provinces.len(), 35, "alias {alias:?}");
        assert_eq!(options.cities[0].name, "全部", "alias {alias:?}");
        assert_eq!(
            options.cities[0].provider_object_id, None,
            "alias {alias:?}"
        );
        assert_eq!(options.districts[0].name, "全部", "alias {alias:?}");
        assert_eq!(
            options.districts[0].provider_object_id, None,
            "alias {alias:?}"
        );
    }
}

#[test]
fn global_nominatim_query_is_exact_and_never_china_scoped() {
    let request = NominatimQuery::for_global("California", 8);
    assert_eq!(request.query, "California");
    assert!(!request.query.contains("China"));
    assert_eq!(request.limit, 8);
    assert_eq!(request.params()["format"], "jsonv2");
    assert_eq!(request.params()["polygon_geojson"], "0");
    assert_eq!(request.params()["addressdetails"], "1");
    assert!(!request.params().contains_key("countrycodes"));
}

#[test]
fn china_fallback_nominatim_query_is_country_restricted_and_candidate_only() {
    let request = NominatimQuery::for_china("巴东县 恩施土家族苗族自治州 湖北省", 8);
    assert_eq!(request.limit, 8);
    assert_eq!(request.params()["countrycodes"], "cn");
    assert_eq!(request.params()["format"], "jsonv2");
    assert_eq!(request.params()["polygon_geojson"], "0");
}

#[test]
fn production_nominatim_uses_the_reachable_official_osm_alias() {
    assert_eq!(NOMINATIM_SEARCH_URL, "https://nominatim.osm.org/search",);
}

#[test]
fn polygon_and_multipolygon_are_normalized_while_point_is_rejected() {
    let polygon = json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]
    });
    let normalized = normalize_polygonal_geometry(polygon).expect("polygon is admissible");
    assert_eq!(normalized.geometry_kind, "polygon");
    assert_eq!(normalized.bbox, [100.0, 30.0, 101.0, 31.0]);

    let multipolygon = json!({
        "type": "MultiPolygon",
        "coordinates": [[[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 30.0]]]]
    });
    let normalized =
        normalize_polygonal_geometry(multipolygon).expect("multipolygon is admissible");
    assert_eq!(normalized.geometry_kind, "multipolygon");

    let point = json!({"type": "Point", "coordinates": [100.0, 30.0]});
    assert!(normalize_polygonal_geometry(point).is_err());
}

#[test]
fn nominatim_discovers_candidates_and_resolves_the_selected_geometry() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        for request_index in 0..2 {
            let (mut stream, _) = listener.accept().expect("one Nominatim request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("request bytes");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).expect("HTTP request is UTF-8");
            assert!(request
                .to_ascii_lowercase()
                .contains("user-agent: eostage/"));
            let body = if request_index == 0 {
                assert!(request.contains("GET /search?"));
                assert!(request.contains("q=California"));
                assert!(request.contains("format=jsonv2"));
                assert!(request.contains("polygon_geojson=0"));
                assert!(request.contains("addressdetails=1"));
                assert!(request.contains("limit=2"));
                serde_json::to_string(&json!([{
                    "place_id": 456,
                    "osm_id": "123",
                    "osm_type": "Relation",
                    "display_name": "California, United States",
                    "name": "California",
                    "type": "administrative",
                    "address": {"country_code": "us", "state": "California"},
                    "boundingbox": [32.0, "42.0", "-124.0", -114.0]
                }]))
                .expect("candidate JSON")
            } else {
                assert!(request.contains("GET /lookup?"));
                assert!(request.contains("osm_ids=R123"));
                assert!(request.contains("format=geojson"));
                assert!(request.contains("polygon_geojson=1"));
                serde_json::to_string(&json!({
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "properties": {
                            "osm_id": "123",
                            "osm_type": "relation",
                            "display_name": "California, United States",
                            "name": "California",
                            "address": {"country_code": "us", "state": "California"}
                        },
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[[-124.0, 32.0], [-114.0, 32.0], [-114.0, 42.0], [-124.0, 32.0]]]
                        }
                    }]
                }))
                .expect("detail JSON")
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response bytes");
        }
    });

    let provider =
        NominatimBoundaryProvider::with_test_endpoint(&format!("http://{address}/search"))
            .expect("loopback provider");
    let candidates = provider
        .discover("California", 2)
        .expect("bounded discovery");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].provider_object_id, "relation:123");
    assert_eq!(candidates[0].bbox, [-124.0, 32.0, -114.0, 42.0]);
    assert_eq!(candidates[0].country_code.as_deref(), Some("US"));
    assert_eq!(
        candidates[0].attribution.as_deref(),
        Some("© OpenStreetMap contributors；许可：ODbL 1.0")
    );
    let result = provider
        .resolve(&candidates[0].provider_object_id)
        .expect("selected polygon resolution");
    assert_eq!(result.provider_object_id, "relation:123");
    assert_eq!(result.geometry_kind, "polygon");
    assert_eq!(result.bbox, [-124.0, 32.0, -114.0, 42.0]);
    assert!(result.geometry.is_object());
    server.join().expect("loopback server completes");
}

#[test]
fn datav_adapter_preserves_china_hierarchy_and_returns_real_geometry() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().expect("one DataV request");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2048];
            loop {
                let read = stream.read(&mut buffer).expect("request bytes");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let line = String::from_utf8(request).expect("HTTP request is UTF-8");
            let path = line.split_whitespace().nth(1).expect("request path");
            let body = match path {
                "/420000_full.json" => json!({
                    "features": [{"properties": {"adcode": "422800", "name": "恩施土家族苗族自治州"}}]
                }),
                "/422800_full.json" => json!({
                    "features": [{"properties": {"adcode": "422823", "name": "巴东县"}}]
                }),
                "/422823.json" => json!({
                    "type": "FeatureCollection",
                    "features": [{
                        "type": "Feature",
                        "properties": {"name": "巴东县"},
                        "geometry": {
                            "type": "Polygon",
                            "coordinates": [[[109.0, 30.0], [110.0, 30.0], [110.0, 31.0], [109.0, 30.0]]]
                        }
                    }]
                }),
                other => panic!("unexpected DataV path: {other}"),
            };
            let body = serde_json::to_string(&body).expect("response JSON");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response bytes");
        }
    });

    let provider = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV provider");
    let options = provider
        .options("湖北省", "恩施土家族苗族自治州")
        .expect("DataV options");
    assert!(options.cities.iter().any(|option| {
        option.name == "恩施土家族苗族自治州"
            && option.provider_object_id.as_deref() == Some("422800")
    }));
    assert!(options.districts.iter().any(|option| {
        option.name == "巴东县" && option.provider_object_id.as_deref() == Some("422823")
    }));

    let results = provider
        .search("湖北省", "恩施土家族苗族自治州", "巴东县", "", 1)
        .expect("DataV boundary");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].provider_object_id, "422823");
    assert_eq!(
        results[0].display_name,
        "湖北省 / 恩施土家族苗族自治州 / 巴东县"
    );
    assert_eq!(results[0].country_code.as_deref(), Some("CN"));
    assert_eq!(results[0].geometry_kind, "polygon");
    assert_ne!(
        results[0].bbox[0], 109.0,
        "DataV GCJ-02 input is normalized"
    );
    server.join().expect("loopback DataV server completes");
}

#[test]
fn datav_taiwan_selector_drops_only_the_redundant_wrapper_and_binds_exact_child_identity() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener
        .set_nonblocking(true)
        .expect("loopback listener becomes nonblocking");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let mut idle_since = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    idle_since = Instant::now();
                    let request = read_bounded_http_request(&mut stream);
                    let line = String::from_utf8(request).expect("HTTP request is UTF-8");
                    let path = line.split_whitespace().nth(1).expect("request path");
                    let body = match path {
                        "/710000_full.json" => json!({
                            "features": [{"properties": {"adcode": "719900", "name": "台湾省"}}]
                        }),
                        "/719900_full.json" => json!({
                            "features": [
                                {"properties": {"adcode": "710100", "name": "台北市"}},
                                {"properties": {"adcode": "710800", "name": "南投县"}}
                            ]
                        }),
                        "/710800.json" => json!({
                            "type": "FeatureCollection",
                            "features": [{
                                "type": "Feature",
                                "properties": {"adcode": "710800", "name": "南投县"},
                                "geometry": {
                                    "type": "Polygon",
                                    "coordinates": [[[120.5, 23.5], [121.0, 23.5], [121.0, 24.0], [120.5, 23.5]]]
                                }
                            }]
                        }),
                        other => panic!("unexpected DataV path: {other}"),
                    };
                    let body = serde_json::to_string(&body).expect("response JSON");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("response bytes");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if idle_since.elapsed() >= Duration::from_millis(150) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            }
        }
    });

    let provider = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV provider");
    let options = provider
        .options("台湾省", "南投县")
        .expect("Taiwan options");
    assert_eq!(
        options.cities,
        vec![
            insar_provider::boundary::ChinaAdminOption::unidentified("全部"),
            insar_provider::boundary::ChinaAdminOption::identified("台北市", "710100"),
            insar_provider::boundary::ChinaAdminOption::identified("南投县", "710800"),
        ]
    );
    assert_eq!(
        options.districts.len(),
        1,
        "Taiwan selector three is unused"
    );
    assert!(!options.cities.iter().any(|option| option.name == "台湾省"));

    let results = provider
        .search("台湾省", "南投县", "全部", "", 1)
        .expect("Taiwan county boundary");
    assert_eq!(results[0].provider_object_id, "710800");
    assert_eq!(results[0].admin_level.as_deref(), Some("city"));
    assert_eq!(results[0].geometry_kind, "polygon");
    server.join().expect("loopback server completes");
}

fn assert_datav_special_region_two_level(
    province_code: &str,
    province_name: &str,
    child_code: &str,
    child_name: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener
        .set_nonblocking(true)
        .expect("loopback listener becomes nonblocking");
    let address = listener.local_addr().expect("loopback address");
    let province_code_for_server = province_code.to_owned();
    let child_code_for_server = child_code.to_owned();
    let child_name_for_server = child_name.to_owned();
    let server = thread::spawn(move || {
        let full_path = format!("/{province_code_for_server}_full.json");
        let geometry_path = format!("/{child_code_for_server}.json");
        let mut paths = Vec::new();
        let mut idle_since = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    idle_since = Instant::now();
                    let request = read_bounded_http_request(&mut stream);
                    let line = String::from_utf8(request).expect("HTTP request is UTF-8");
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .expect("request path")
                        .to_owned();
                    paths.push(path.clone());
                    let body = if path == full_path {
                        json!({
                            "features": [{
                                "properties": {
                                    "adcode": child_code_for_server,
                                    "name": child_name_for_server
                                }
                            }]
                        })
                    } else if path == geometry_path {
                        json!({
                            "type": "FeatureCollection",
                            "features": [{
                                "type": "Feature",
                                "properties": {
                                    "adcode": child_code_for_server,
                                    "name": child_name_for_server
                                },
                                "geometry": {
                                    "type": "Polygon",
                                    "coordinates": [[[113.5, 22.1], [114.0, 22.1], [114.0, 22.5], [113.5, 22.1]]]
                                }
                            }]
                        })
                    } else {
                        panic!("two-level special region requested an unexpected path: {path}");
                    };
                    let body = serde_json::to_string(&body).expect("response JSON");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("response bytes");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if idle_since.elapsed() >= Duration::from_millis(150) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            }
        }
        paths
    });

    let provider = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV provider");
    let options = provider
        .options(province_name, child_name)
        .expect("special-region options");
    assert_eq!(options.cities.len(), 2, "全部 plus exact DataV child");
    assert_eq!(options.cities[1].name, child_name);
    assert_eq!(
        options.cities[1].provider_object_id.as_deref(),
        Some(child_code)
    );
    assert_eq!(
        options.districts.len(),
        1,
        "special-region selector three is unused"
    );

    let results = provider
        .search(province_name, child_name, "全部", "", 1)
        .expect("special-region child boundary");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].provider_object_id, child_code);
    assert_eq!(results[0].admin_level.as_deref(), Some("district"));
    assert_eq!(results[0].geometry_kind, "polygon");

    let paths = server.join().expect("loopback server completes");
    assert_eq!(
        paths,
        vec![
            format!("/{province_code}_full.json"),
            format!("/{province_code}_full.json"),
            format!("/{child_code}.json"),
        ],
        "DataV special-region selector must not discover or request a third level"
    );
}

#[test]
fn datav_hong_kong_and_macao_use_exact_direct_second_level_adcodes() {
    assert_datav_special_region_two_level("810000", "香港特别行政区", "810002", "湾仔区");
    assert_datav_special_region_two_level("820000", "澳门特别行政区", "820001", "花地玛堂区");
}

#[test]
fn datav_selector_scope_returns_the_exact_requested_administrative_level() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener
        .set_nonblocking(true)
        .expect("loopback listener becomes nonblocking");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let mut paths = Vec::new();
        let mut idle_since = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    idle_since = Instant::now();
                    let request = read_bounded_http_request(&mut stream);
                    let line = String::from_utf8(request).expect("HTTP request is UTF-8");
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .expect("request path")
                        .to_owned();
                    paths.push(path.clone());
                    let body = match path.as_str() {
                        "/530000_full.json" => json!({
                            "features": [{"properties": {"adcode": 530100, "name": "昆明市"}}]
                        }),
                        "/530100_full.json" => json!({
                            "features": [{"properties": {"adcode": 530102, "name": "五华区"}}]
                        }),
                        "/530000.json" | "/530100.json" | "/530102.json" => json!({
                            "type": "FeatureCollection",
                            "features": [{
                                "type": "Feature",
                                "properties": {"name": "fixture"},
                                "geometry": {
                                    "type": "Polygon",
                                    "coordinates": [[[102.0, 24.0], [103.0, 24.0], [103.0, 25.0], [102.0, 24.0]]]
                                }
                            }]
                        }),
                        other => panic!("unexpected DataV path: {other}"),
                    };
                    let body = serde_json::to_string(&body).expect("response JSON");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("response bytes");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if !paths.is_empty() && idle_since.elapsed() > Duration::from_millis(100) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            }
        }
        paths
    });

    let provider = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV provider");
    let province = provider
        .search("云南省", "全部", "全部", "", 8)
        .expect("province boundary");
    let city = provider
        .search("云南省", "昆明市", "全部", "", 8)
        .expect("city boundary");
    let district = provider
        .search("云南省", "昆明市", "五华区", "", 8)
        .expect("district boundary");
    let paths = server.join().expect("loopback server completes");

    assert_eq!(province.len(), 1);
    assert_eq!(province[0].provider_object_id, "530000");
    assert_eq!(province[0].admin_level.as_deref(), Some("province"));
    assert_eq!(city.len(), 1);
    assert_eq!(city[0].provider_object_id, "530100");
    assert_eq!(city[0].admin_level.as_deref(), Some("city"));
    assert_eq!(district.len(), 1);
    assert_eq!(district[0].provider_object_id, "530102");
    assert_eq!(district[0].admin_level.as_deref(), Some("district"));
    assert_eq!(
        paths,
        vec![
            "/530000.json",
            "/530000_full.json",
            "/530100.json",
            "/530000_full.json",
            "/530100_full.json",
            "/530102.json",
        ]
    );
}

#[test]
fn datav_municipalities_expose_exact_districts_as_selector_two_without_a_third_level() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
    listener
        .set_nonblocking(true)
        .expect("loopback listener becomes nonblocking");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        let mut paths = Vec::new();
        let mut idle_since = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    idle_since = Instant::now();
                    let request = read_bounded_http_request(&mut stream);
                    let line = String::from_utf8(request).expect("HTTP request is UTF-8");
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .expect("request path")
                        .to_owned();
                    paths.push(path.clone());
                    let body = match path.as_str() {
                        "/110000_full.json" => {
                            json!({"features": [{"properties": {"adcode": 110108, "name": "海淀区"}}]})
                        }
                        "/120000_full.json" => {
                            json!({"features": [{"properties": {"adcode": 120100, "name": "天津市"}}]})
                        }
                        "/120100_full.json" => {
                            json!({"features": [{"properties": {"adcode": 120116, "name": "滨海新区"}}]})
                        }
                        "/310000_full.json" => {
                            json!({"features": [{"properties": {"adcode": 310115, "name": "浦东新区"}}]})
                        }
                        "/500000_full.json" => {
                            json!({"features": [{"properties": {"adcode": 500112, "name": "渝北区"}}]})
                        }
                        "/110108.json" => json!({
                            "type": "FeatureCollection",
                            "features": [{
                                "type": "Feature",
                                "properties": {"name": "海淀区", "adcode": 110108},
                                "geometry": {
                                    "type": "Polygon",
                                    "coordinates": [[[116.0, 39.8], [116.4, 39.8], [116.4, 40.1], [116.0, 39.8]]]
                                }
                            }]
                        }),
                        other => {
                            panic!("municipality selector must not request a third level: {other}")
                        }
                    };
                    let body = serde_json::to_string(&body).expect("response JSON");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .expect("response bytes");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if !paths.is_empty() && idle_since.elapsed() > Duration::from_millis(100) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("loopback accept failed: {error}"),
            }
        }
        paths
    });

    let provider = DataVBoundaryProvider::with_test_root(&format!("http://{address}/"))
        .expect("loopback DataV provider");
    for (province, district, adcode) in [
        ("北京市", "海淀区", "110108"),
        ("天津市", "滨海新区", "120116"),
        ("上海市", "浦东新区", "310115"),
        ("重庆市", "渝北区", "500112"),
    ] {
        let options = provider
            .options(province, district)
            .expect("municipality options");
        assert!(options.cities.iter().any(|option| {
            option.name == district && option.provider_object_id.as_deref() == Some(adcode)
        }));
        assert_eq!(options.districts.len(), 1);
        assert_eq!(options.districts[0].name, "全部");
    }

    let result = provider
        .search("北京市", "海淀区", "全部", "", 8)
        .expect("municipality district boundary");
    let paths = server.join().expect("loopback server completes");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].provider_object_id, "110108");
    assert_eq!(result[0].admin_level.as_deref(), Some("district"));
    assert_eq!(result[0].display_name, "北京市 / 海淀区");
    assert_eq!(
        paths,
        vec![
            "/110000_full.json",
            "/120000_full.json",
            "/120100_full.json",
            "/310000_full.json",
            "/500000_full.json",
            "/110000_full.json",
            "/110108.json",
        ]
    );
}
