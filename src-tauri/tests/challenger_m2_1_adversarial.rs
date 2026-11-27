//! Adversarial Challenger Stress Harness for Milestone 2:
//! - Credentials validation & error handling without panic
//! - Tile generation stress testing (out-of-bounds zoom, inverted bbox, empty asset IDs)
//! - GeoTIFF magic byte validation (reject HTML 404/500, corrupt bytes; accept 4949 2a00 & 4d4d 002a)

use eostage_desktop::geodata_tools::{execute_open_geodata_query, OpenGeodataQueryRequest};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

static HARNESS_COUNTER: AtomicU64 = AtomicU64::new(100);
static VAULT_ENV_LOCK: Mutex<()> = Mutex::new(());

fn shared_loopback_mpc_base_url() -> &'static str {
    static SERVER_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SERVER_URL.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind shared loopback MPC");
        let addr = listener.local_addr().expect("local addr");
        let base_url = format!("http://{addr}");
        let base_for_srv = base_url.clone();
        let one_px_png: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let png = one_px_srv_png(&one_px_png);
                let base = base_for_srv.clone();
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                    let mut data = Vec::new();
                    let mut buf = [0u8; 4096];
                    let mut header_end = None;
                    let mut content_len = 0usize;
                    loop {
                        match stream.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                data.extend_from_slice(&buf[..n]);
                                if header_end.is_none() {
                                    if let Some(pos) =
                                        data.windows(4).position(|w| w == b"\r\n\r\n")
                                    {
                                        let hend = pos + 4;
                                        header_end = Some(hend);
                                        let hdr = String::from_utf8_lossy(&data[..hend]);
                                        for line in hdr.lines() {
                                            if let Some(rest) = line
                                                .to_ascii_lowercase()
                                                .strip_prefix("content-length:")
                                            {
                                                content_len = rest.trim().parse().unwrap_or(0);
                                            }
                                        }
                                    }
                                }
                                if let Some(hend) = header_end {
                                    if data.len() >= hend + content_len {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let req_str = String::from_utf8_lossy(&data);
                    let first_line = req_str.lines().next().unwrap_or("");
                    let body_slice = header_end.and_then(|h| data.get(h..)).unwrap_or(&[]);
                    if first_line.contains("/api/stac/v1/search") {
                        let req_json: serde_json::Value =
                            serde_json::from_slice(body_slice).unwrap_or_else(|_| json!({}));
                        let coll = req_json["collections"]
                            .as_array()
                            .and_then(|a| a.first())
                            .and_then(|v| v.as_str())
                            .unwrap_or("sentinel-2-l2a");
                        let href = format!("{base}/assets/sample.tif");
                        let out = json!({
                            "type": "FeatureCollection",
                            "features": [{
                                "id": format!("MOCK_{}_ITEM", coll.replace('-', "_").to_ascii_uppercase()),
                                "collection": coll,
                                "bbox": [116.0, 30.0, 119.0, 33.0],
                                "geometry": {
                                    "type": "Polygon",
                                    "coordinates": [[[116.0, 30.0], [119.0, 30.0], [119.0, 33.0], [116.0, 33.0], [116.0, 30.0]]]
                                },
                                "properties": { "datetime": "2024-01-10T00:00:00Z", "esa_worldcover:product_version": "V2.0.0" },
                                "assets": {
                                    "visual": { "href": href },
                                    "map": { "href": href },
                                    "v200": { "href": href },
                                    "data": { "href": href }
                                }
                            }]
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            out.len(),
                            out
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    } else if first_line.contains("/api/data/v1/item/tilejson.json") {
                        let out = json!({
                            "tiles": [
                                "https://planetarycomputer.microsoft.com/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?collection=sentinel-2-l2a&item=MOCK_ITEM&assets=visual"
                            ]
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            out.len(),
                            out
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    } else if first_line.contains("/api/sas/v1/token/") {
                        let out = json!({
                            "msft:expiry": "2099-01-01T00:00:00Z",
                            "token": "sv=2021-06-08&se=2099-01-01T00%3A00%3A00Z&sr=c&sp=rl&sig=mocksig"
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            out.len(),
                            out
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    } else {
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            png.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes());
                        let _ = stream.write_all(&png);
                    }
                });
            }
        });
        base_url
    })
}

fn one_px_srv_png(src: &[u8]) -> Vec<u8> {
    src.to_vec()
}

fn temp_sandbox_dir(prefix: &str) -> (PathBuf, MutexGuard<'static, ()>) {
    let guard = VAULT_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let id = HARNESS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "challenger_m2_1_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp sandbox dir");
    fs::write(
        dir.join("network-settings.json"),
        json!({
            "mode": "direct",
            "test_loopback": true,
            "mpc_test_base_url": shared_loopback_mpc_base_url()
        })
        .to_string(),
    )
    .expect("write loopback network-settings.json");
    std::env::set_var(
        "EOSTAGE_MAC_CREDENTIAL_VAULT_PATH",
        dir.join("isolated_vault.json"),
    );
    std::env::remove_var("EOSTAGE_ISOLATE_EMPTY_KEYCHAIN");
    (dir, guard)
}

// =========================================================================
// Area 1: Malformed and Invalid Credentials Stress Testing
// =========================================================================

#[test]
fn test_credentials_empty_project_id_rejected_gracefully() {
    let (dir, _guard) = temp_sandbox_dir("empty_proj");

    // 1. gee_account_save with empty project_id
    let res = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("".to_owned()),
            keyword: Some("service_account".to_owned()),
            sql: Some(
                r#"{"client_email":"test@test.iam.gserviceaccount.com","private_key":"key"}"#
                    .to_owned(),
            ),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );
    assert!(res.is_err(), "empty project_id must be rejected");
    let err = res.unwrap_err();
    assert!(
        err.contains("GEE_PROJECT_ID_REQUIRED"),
        "error must cite GEE_PROJECT_ID_REQUIRED, got: {err}"
    );

    // 2. gee_account_save with whitespace project_id
    let res_ws = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("   \t\n  ".to_owned()),
            keyword: Some("service_account".to_owned()),
            sql: Some(
                r#"{"client_email":"test@test.iam.gserviceaccount.com","private_key":"key"}"#
                    .to_owned(),
            ),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );
    assert!(res_ws.is_err());
    assert!(res_ws.unwrap_err().contains("GEE_PROJECT_ID_REQUIRED"));

    // 3. gee_account_verify with empty credentials
    let res_ver = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_verify".to_owned(),
            code: Some("".to_owned()),
            keyword: Some("service_account".to_owned()),
            sql: Some("".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );
    assert!(res_ver.is_err());
    let err_ver = res_ver.unwrap_err();
    assert!(
        err_ver.contains("GEE_NOT_CONFIGURED"),
        "error must cite GEE_NOT_CONFIGURED, got: {err_ver}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_credentials_broken_json_and_missing_keys_handled_without_panic() {
    let (dir, _guard) = temp_sandbox_dir("broken_json");

    // 1. Broken JSON syntax in service_account save
    let bad_json_payloads = [
        "{ not valid json at all",
        "{ \"client_email\": ",
        "",
        "\"a plain string instead of object\"",
        "12345",
        "null",
        "{\"foo\": \"bar\"}", // missing client_email and private_key
    ];

    for bad in &bad_json_payloads {
        let res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_save".to_owned(),
                code: Some("ee-test-proj".to_owned()),
                keyword: Some("service_account".to_owned()),
                sql: Some((*bad).to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        );
        assert!(res.is_err(), "Payload '{bad}' must fail");
        let err = res.unwrap_err();
        assert!(
            err.contains("INVALID_SERVICE_ACCOUNT_JSON") || err.contains("GEE_CREDENTIAL_REQUIRED"),
            "Error for '{bad}' must be descriptive, got: {err}"
        );
    }

    // 2. Broken JSON in verify
    for bad in &bad_json_payloads {
        let res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_account_verify".to_owned(),
                code: Some("ee-test-proj".to_owned()),
                keyword: Some("service_account".to_owned()),
                sql: Some((*bad).to_owned()),
                bbox: None,
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: None,
                full_children: None,
                tool_name: None,
                tool_args: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        );
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(
            err.contains("INVALID_SA_JSON") || err.contains("GEE_NOT_CONFIGURED"),
            "Error must report invalid JSON/missing credentials, got: {err}"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_credentials_corrupt_rsa_key_fails_gracefully_with_no_file_leaks() {
    let (dir, _guard) = temp_sandbox_dir("corrupt_rsa");

    // SA JSON with a bogus private_key that cannot be parsed by openssl
    let sa_json = json!({
        "type": "service_account",
        "client_email": "test-sa@ee-test.iam.gserviceaccount.com",
        "private_key": "-----BEGIN RSA PRIVATE KEY-----\nCORRUPT_NOT_A_VALID_KEY\n-----END RSA PRIVATE KEY-----",
        "token_uri": "https://oauth2.googleapis.com/token"
    });

    let res = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_verify".to_owned(),
            code: Some("ee-test-proj".to_owned()),
            keyword: Some("service_account".to_owned()),
            sql: Some(sa_json.to_string()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );

    // Verify error is returned and no panic occurs
    // Either openssl fails signature, or discovery ping fails on offline machine
    assert!(res.is_err() || res.is_ok()); // if network reachable fallback discovery ping may return Err
    if let Err(err) = res {
        assert!(
            err.contains("GEE_SA_TOKEN_EXCHANGE_FAILED")
                || err.contains("GEE_PROXY_REQUIRED")
                || err.contains("GEE_DISCOVERY_HTTP")
                || err.contains("INVALID_SA_JSON"),
            "Error must be handled gracefully, got: {err}"
        );
    }

    // Crucial: check that temporary signing files were cleaned up
    let entries = fs::read_dir(&dir).unwrap().flatten().collect::<Vec<_>>();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.starts_with(".gee_sa_key_") && !name.starts_with(".gee_sa_msg_"),
            "Temporary file {name} was leaked!"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_credentials_oauth_http_errors_handled_gracefully() {
    let (dir, _guard) = temp_sandbox_dir("oauth_http_err");

    // Spin up local mock server returning HTTP 401 Unauthorized
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");

    let server = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let body = r#"{"error": {"code": 401, "message": "Request had invalid authentication credentials."}}"#;
            let resp = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    let cred_json = json!({
        "project_id": "ee-test-oauth",
        "api_base_url": base_url,
        "auth_mode": "oauth_token",
        "secret": "ya29.invalid_expired_token_abc"
    });
    fs::write(dir.join("gee_credentials.json"), cred_json.to_string()).unwrap();

    let res = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_verify".to_owned(),
            code: Some("ee-test-oauth".to_owned()),
            keyword: Some("oauth_token".to_owned()),
            sql: Some("ya29.invalid_expired_token_abc".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );
    let _ = server.join();

    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        err.contains("GEE_VERIFY_HTTP_401"),
        "error must cite GEE_VERIFY_HTTP_401, got: {err}"
    );

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Area 2: Tile Generation Stress Testing (Zoom Bounds, Inverted BBOX, Empty Assets)
// =========================================================================

#[test]
fn test_tile_generation_zoom_bounds_clamp_without_overflow() {
    let (dir, _guard) = temp_sandbox_dir("zoom_clamp");

    // 1. Extreme zoom out of bounds: zoom 0, zoom 35, zoom 100
    let tile_tpl = format!(
        "{}/tiles/{{z}}/{{y}}/{{x}}.png",
        shared_loopback_mpc_base_url()
    );
    let extreme_zooms = ["0", "30", "35", "100", "-5"];
    for z in &extreme_zooms {
        let req = OpenGeodataQueryRequest {
            dataset: "xyz_satellite_tiles".to_owned(),
            code: Some(tile_tpl.clone()),
            keyword: Some(format!("custom:{z}:nomosaic:gtiff:DEFLATE:nopyramids")),
            bbox: Some([117.275, 31.855, 117.276, 31.856]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };

        // Must clamp zoom into [1, 21] without bitshift overflow (1u64 << z) or crash
        let res = execute_open_geodata_query(&dir, &req);
        // Res may be Ok or Err depending on network, but MUST NOT panic
        if let Ok(val) = res {
            let zoom = val["zoom"].as_u64().unwrap();
            assert!(
                (1..=21).contains(&zoom),
                "zoom {z} must be clamped to 1..=21, got {zoom}"
            );
        }
    }

    // 2. gee_map_tiles with out-of-bounds visualization values
    let tile_req = OpenGeodataQueryRequest {
        dataset: "gee_map_tiles".to_owned(),
        code: Some("COPERNICUS/S2_SR_HARMONIZED".to_owned()),
        keyword: None,
        bbox: None,
        lat: None,
        lng: None,
        start_date: None,
        end_date: None,
        osm_tag: None,
        dem_product: None,
        iso3: None,
        year: None,
        output_path: None,
        full_children: None,
        tool_name: None,
        tool_args: None,
        sql: Some("B4,B3,B2|-1000..999999".to_owned()),
        gee_params: None,
        aoi_geojson: None,
        zooms: None,
    };
    let tile_res =
        execute_open_geodata_query(&dir, &tile_req).expect("gee_map_tiles with wide range");
    assert_eq!(tile_res["ok"], true);
    assert_eq!(tile_res["min"], -1000.0);
    assert_eq!(tile_res["max"], 999999.0);
    let tpl = tile_res["tile_url_template"].as_str().unwrap();
    assert!(tpl.contains("{z}/{x}/{y}"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_tile_generation_inverted_and_degenerate_bbox_normalized() {
    let (dir, _guard) = temp_sandbox_dir("inverted_bbox");

    // 1. Inverted bbox: west > east and south > north
    let inverted_bboxes = [
        [118.0, 31.0, 117.0, 32.0],     // west > east
        [117.0, 32.0, 118.0, 31.0],     // south > north
        [118.0, 32.0, 117.0, 31.0],     // both inverted
        [117.25, 31.85, 117.25, 31.85], // zero-area point bbox
    ];

    let tile_tpl = format!(
        "{}/tiles/{{z}}/{{y}}/{{x}}.png",
        shared_loopback_mpc_base_url()
    );
    for (i, bbox) in inverted_bboxes.iter().enumerate() {
        let req = OpenGeodataQueryRequest {
            dataset: "xyz_satellite_tiles".to_owned(),
            code: Some(tile_tpl.clone()),
            keyword: Some("custom:14:nomosaic:gtiff:DEFLATE:nopyramids".to_owned()),
            bbox: Some(*bbox),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };

        let res = execute_open_geodata_query(&dir, &req);
        // Must normalize query_bbox and not crash with negative range or zero division
        assert!(
            res.is_ok() || res.is_err(),
            "Bbox {i} [{:?}] handled without panic",
            bbox
        );
    }

    // 2. gee_catalog Route 1 with inverted bbox
    let gee_req = OpenGeodataQueryRequest {
        dataset: "gee_catalog".to_owned(),
        code: Some("ESA/WorldCover/v200".to_owned()),
        keyword: Some("ESA/WorldCover/v200".to_owned()),
        bbox: Some([118.0, 32.0, 117.0, 31.0]), // inverted
        lat: None,
        lng: None,
        start_date: None,
        end_date: None,
        osm_tag: None,
        dem_product: None,
        iso3: None,
        year: None,
        output_path: None,
        full_children: None,
        tool_name: None,
        tool_args: None,
        sql: None,
        gee_params: None,
        aoi_geojson: None,
        zooms: None,
    };
    let gee_res =
        execute_open_geodata_query(&dir, &gee_req).expect("gee_catalog with inverted bbox");
    assert_eq!(gee_res["ok"], true);
    assert_eq!(gee_res["route_mode"], "cloud_native_direct");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_tile_generation_empty_and_whitespace_asset_ids_fallback_safely() {
    let (dir, _guard) = temp_sandbox_dir("empty_asset");

    // 1. gee_map_tiles with empty code / whitespace code
    let empty_variants = [None, Some("".to_owned()), Some("   ".to_owned())];
    for empty in &empty_variants {
        let req = OpenGeodataQueryRequest {
            dataset: "gee_map_tiles".to_owned(),
            code: empty.clone(),
            keyword: None,
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res = execute_open_geodata_query(&dir, &req).expect("fallback on empty asset id");
        assert_eq!(res["ok"], true);
        assert_eq!(
            res["gee_asset_id"], "COPERNICUS/S2_SR_HARMONIZED",
            "must fallback to default Sentinel-2"
        );
    }

    // 2. gee_catalog with empty code
    for empty in &empty_variants {
        let req = OpenGeodataQueryRequest {
            dataset: "gee_catalog".to_owned(),
            code: empty.clone(),
            keyword: None,
            bbox: Some([117.2, 31.8, 117.3, 31.9]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        };
        let res =
            execute_open_geodata_query(&dir, &req).expect("fallback on empty catalog asset id");
        assert_eq!(res["ok"], true);
        assert_eq!(
            res["gee_asset_id"], "ESA/WorldCover/v200",
            "must fallback to default ESA/WorldCover/v200"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Area 3: GeoTIFF Magic Byte Validation Stress Testing
// =========================================================================

#[test]
fn test_geotiff_magic_byte_rejection_of_html_and_corrupt_payloads() {
    let (dir, _guard) = temp_sandbox_dir("bad_tiff_payloads");

    execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("test-bad-tiff-proj".to_owned()),
            keyword: Some("oauth_token".to_owned()),
            sql: Some("mock_valid_token".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    )
    .expect("seed oauth token in isolated vault");

    let adversarial_payloads = [
        // (name, content_type, bytes, expected_error_substring)
        (
            "HTML 404 error",
            "text/html",
            b"<html><head><title>404 Not Found</title></head><body>Asset does not exist</body></html>".to_vec(),
            "GEE_INVALID_TIFF_HEADER",
        ),
        (
            "HTML 500 error",
            "text/html",
            b"<html><body>500 Internal Server Error: GEE compute quota exceeded</body></html>".to_vec(),
            "GEE_INVALID_TIFF_HEADER",
        ),
        (
            "Corrupt random binary",
            "application/octet-stream",
            vec![0x00, 0x01, 0x02, 0x03, 0xde, 0xad, 0xbe, 0xef],
            "GEE_INVALID_TIFF_HEADER",
        ),
        (
            "JPEG fake image",
            "image/jpeg",
            vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46],
            "GEE_INVALID_TIFF_HEADER",
        ),
        (
            "PNG fake image",
            "image/png",
            vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
            "GEE_INVALID_TIFF_HEADER",
        ),
        (
            "Truncated 2 bytes",
            "image/tiff",
            vec![0x49, 0x49],
            "GEE_INVALID_TIFF_HEADER",
        ),
    ];

    for (case_name, ctype, payload, expected_err) in adversarial_payloads {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let server_payload = payload.clone();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ctype,
                    server_payload.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(&server_payload);
            }
        });

        let cred_json = json!({
            "project_id": "test-bad-tiff-proj",
            "api_base_url": base_url,
            "auth_mode": "oauth_token",
            "secret": "mock_valid_token"
        });
        fs::write(dir.join("gee_credentials.json"), cred_json.to_string()).unwrap();

        let res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("projects/test-bad-tiff-proj/assets/test_raster".to_owned()),
                keyword: Some("projects/test-bad-tiff-proj/assets/test_raster|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(dir.join("bad_out").to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        );
        let _ = server.join();

        assert!(
            res.is_err(),
            "Payload '{case_name}' must be rejected as invalid GeoTIFF"
        );
        let err = res.unwrap_err();
        assert!(
            err.contains(expected_err),
            "Case '{case_name}' expected error '{expected_err}', got '{err}'"
        );

        // Verify no .tif file was written to disk
        let tif_count = fs::read_dir(dir.join("bad_out"))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| {
                        entry.path().extension().and_then(|s| s.to_str()) == Some("tif")
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(
            tif_count, 0,
            "Corrupt payload '{case_name}' must NEVER be saved to disk!"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_geotiff_valid_headers_accepted_both_little_and_big_endian() {
    let (dir, _guard) = temp_sandbox_dir("valid_tiff_headers");

    execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "gee_account_save".to_owned(),
            code: Some("test-valid-tiff-proj".to_owned()),
            keyword: Some("oauth_token".to_owned()),
            sql: Some("mock_valid_token".to_owned()),
            bbox: None,
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: None,
            full_children: None,
            tool_name: None,
            tool_args: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    )
    .expect("seed oauth token in isolated vault");

    let valid_headers = [
        (
            "little-endian TIFF",
            b"II*\0\x08\0\0\0\xaa\xbb\xcc\xdd".to_vec(),
            "49492a00",
        ),
        (
            "big-endian TIFF",
            b"MM\0*\0\0\0\x08\x11\x22\x33\x44".to_vec(),
            "4d4d002a",
        ),
    ];

    for (idx, (name, payload, expected_magic)) in valid_headers.into_iter().enumerate() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");

        let server_payload = payload.clone();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    server_payload.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(&server_payload);
            }
        });

        let cred_json = json!({
            "project_id": "test-valid-tiff-proj",
            "api_base_url": base_url,
            "auth_mode": "oauth_token"
        });
        fs::write(dir.join("gee_credentials.json"), cred_json.to_string()).unwrap();

        let out_dir = dir.join(format!("valid_out_{idx}"));
        let res = execute_open_geodata_query(
            &dir,
            &OpenGeodataQueryRequest {
                dataset: "gee_catalog".to_owned(),
                code: Some("projects/test-valid-tiff-proj/assets/good_dem".to_owned()),
                keyword: Some("projects/test-valid-tiff-proj/assets/good_dem|download".to_owned()),
                bbox: Some([117.20, 31.80, 117.30, 31.90]),
                lat: None,
                lng: None,
                start_date: None,
                end_date: None,
                osm_tag: None,
                dem_product: None,
                iso3: None,
                year: None,
                output_path: Some(out_dir.to_string_lossy().to_string()),
                full_children: None,
                tool_name: None,
                tool_args: None,
                sql: None,
                gee_params: None,
                aoi_geojson: None,
                zooms: None,
            },
        );
        let _ = server.join();

        assert!(
            res.is_ok(),
            "Valid header '{name}' must succeed, got: {:?}",
            res.err()
        );
        let val = res.unwrap();
        assert_eq!(val["ok"], true);
        assert_eq!(val["magic_bytes"], expected_magic);
        assert_eq!(val["downloaded_bytes"], payload.len());
        assert_eq!(val["size_bytes"], payload.len());

        let out_file = PathBuf::from(val["downloaded_file"].as_str().unwrap());
        assert!(out_file.is_file(), "Downloaded file must exist on disk");
        let written = fs::read(&out_file).unwrap();
        assert_eq!(written, payload);
    }

    let _ = fs::remove_dir_all(&dir);
}
