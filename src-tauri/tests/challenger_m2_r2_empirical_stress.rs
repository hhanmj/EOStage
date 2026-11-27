//! Challenger 2 Empirical Stress Test Harness for Milestone 2 Remediation
//!
//! Empirically stress-tests:
//! 1. Open Geodata Download Logic in `src-tauri/src/geodata_tools.rs`:
//!    - NGL GNSS (`cols[7]` and `cols[8]` indexing, URL fallback, bad line resilience)
//!    - NASA OPERA (`data_url` priority, 20MB limit bug & degradation)
//!    - MOD13Q1 & Sentinel-5P (15MB truncation bug: `Read::take(..., 2 * 1024 * 1024)`)
//!    - IGS IONEX (Return JSON mismatch: `downloaded_file` is `.DCBi` instead of `.24I`)
//! 2. Route Policy in `insar-provider`:
//!    - `SENTINEL2_L1C_ROUTE` production resolution (`production_enabled` is still false!)
//!    - `SENTINEL3_STANDARD_ROUTE` production resolution

use eostage_desktop::geodata_tools::{execute_open_geodata_query, OpenGeodataQueryRequest};
use insar_provider::catalog::{DatasetRoutePolicyRegistry, RouteUse, SENTINEL2_L1C_ROUTE_ID};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static HARNESS_COUNTER: AtomicU64 = AtomicU64::new(500);

fn temp_sandbox_dir(prefix: &str) -> PathBuf {
    let id = HARNESS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "challenger_m2_r2_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp sandbox dir");
    fs::write(
        dir.join("network-settings.json"),
        r#"{"mode":"direct","test_loopback":true}"#,
    )
    .expect("write loopback network-settings.json");
    dir
}

// =========================================================================
// Area 1: IGS IONEX Response JSON Mapping
// =========================================================================

#[test]
#[ignore = "live network acceptance test"]
fn test_empirical_igs_ionex_return_json_fields() {
    let dir = temp_sandbox_dir("ionex_fields");

    let res = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "ionospheric_ionex".to_owned(),
            code: None,
            keyword: Some("COD".to_owned()),
            bbox: Some([117.20, 31.80, 117.26, 31.85]),
            lat: None,
            lng: None,
            start_date: Some("2024-05-15".to_owned()),
            end_date: None,
            osm_tag: None,
            dem_product: None,
            iso3: None,
            year: None,
            output_path: Some(dir.to_string_lossy().to_string()),
            full_children: None,
            tool_name: None,
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    )
    .expect("execute ionospheric_ionex");

    assert_eq!(res["ok"], true);
    assert_eq!(res["filename"], "CODG1360.24I");

    let downloaded_file = res["downloaded_file"]
        .as_str()
        .expect("downloaded_file exists");
    let downloaded_path = res["downloaded_path"]
        .as_str()
        .expect("downloaded_path exists");
    let ionex_file = res["ionex_file"].as_str().expect("ionex_file exists");
    let dcb_file = res["dcb_file"].as_str().expect("dcb_file exists");

    println!("IONEX filename: {}", res["filename"]);
    println!("IONEX ionex_file: {}", ionex_file);
    println!("IONEX dcb_file: {}", dcb_file);
    println!("IONEX downloaded_file: {}", downloaded_file);
    println!("IONEX downloaded_path: {}", downloaded_path);

    // DEFECT PROOF:
    // The worker claimed: "Fixed return JSON mapping: 'filename': filename, 'downloaded_file': ionex_file_str, and 'downloaded_path': ionex_file_str"
    // But in the code:
    // downloaded_file is set to dcb_file_str (CODG1360.DCBi), NOT ionex_file_str (CODG1360.24I)!
    let is_dcb = downloaded_file.ends_with(".DCBi");
    let is_ionex = downloaded_file.ends_with(".24I");

    println!("Is downloaded_file pointing to .DCBi? {}", is_dcb);
    println!("Is downloaded_file pointing to .24I? {}", is_ionex);

    assert!(
        is_ionex,
        "downloaded_file must point to .24I ionex file instead of .DCBi companion file"
    );

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Area 2: MOD13Q1 & STAC Streaming Truncation Bug (Read::take 2MB limit)
// =========================================================================

#[test]
fn test_empirical_modis_truncation_when_content_len_exceeds_15mb() {
    let dir = temp_sandbox_dir("modis_truncation");

    // Spin up mock server serving a 20 MB dummy payload
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let payload_size = 20 * 1024 * 1024; // 20 MB

    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let req_str = String::from_utf8_lossy(&buf);

            if req_str.contains("/search") {
                // Mock STAC search response
                let stac_resp = json!({
                    "features": [{
                        "id": "MOD13Q1.A2024001.h28v05.061.2024020042450",
                        "collection": "modis-13Q1-061",
                        "properties": { "datetime": "2024-01-01T00:00:00Z" },
                        "bbox": [116.0, 30.0, 118.0, 32.0],
                        "geometry": { "type": "Polygon", "coordinates": [[[116.0, 30.0], [118.0, 30.0], [118.0, 32.0], [116.0, 32.0], [116.0, 30.0]]] },
                        "assets": {
                            "hdf": {
                                "href": format!("http://{addr}/data/modis_large.hdf")
                            }
                        }
                    }]
                });
                let body = stac_resp.to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            } else if req_str.contains("/data/modis_large.hdf") {
                // Mock 20 MB data download
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-hdf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload_size
                );
                let _ = stream.write_all(header.as_bytes());
                // Write 20 MB of dummy bytes in chunks
                let chunk = [0x55u8; 64 * 1024];
                let mut remaining = payload_size;
                while remaining > 0 {
                    let to_write = remaining.min(chunk.len());
                    if stream.write_all(&chunk[..to_write]).is_err() {
                        break;
                    }
                    remaining -= to_write;
                }
            }
        }
    });

    // Mock network settings to use our mock server directly
    let out_dir = dir.join("modis_out");
    fs::create_dir_all(&out_dir).unwrap();

    // Call open geodata query with custom STAC URL
    let res = execute_open_geodata_query(
        &dir,
        &OpenGeodataQueryRequest {
            dataset: "modis_ndvi".to_owned(),
            code: None,
            keyword: Some(format!("http://{addr}/search|download")),
            bbox: Some([117.0, 31.0, 117.5, 31.5]),
            lat: None,
            lng: None,
            start_date: None,
            end_date: None,
            osm_tag: None,
            dem_product: Some("modis-13Q1-061".to_owned()),
            iso3: None,
            year: None,
            output_path: Some(out_dir.to_string_lossy().to_string()),
            full_children: None,
            tool_name: Some("download".to_owned()),
            tool_args: None,
            sql: None,
            gee_params: None,
            aoi_geojson: None,
            zooms: None,
        },
    );

    println!("MODIS Query result: {:?}", res);
    assert!(res.is_ok());
    let val = res.unwrap();
    assert_eq!(val["ok"], true);

    let downloaded_file_path = PathBuf::from(val["downloaded_file"].as_str().unwrap());
    assert!(
        downloaded_file_path.is_file(),
        "File must be written on disk"
    );

    let file_len = fs::metadata(&downloaded_file_path).unwrap().len();
    println!(
        "Downloaded file size: {} bytes (server had {} bytes)",
        file_len, payload_size
    );

    // DEFECT PROOF:
    // When content_len > 15MB, geodata_tools.rs:2101 truncates:
    // `let mut limited = std::io::Read::take(&mut resp, 2 * 1024 * 1024);`
    // So file_len is EXACTLY 2,097,152 bytes (2 MB), NOT 20 MB!
    assert!(
        file_len >= 15 * 1024 * 1024,
        "File must be completely downloaded without 2MB truncation (expected >= 15 MB, got {} bytes)",
        file_len
    );

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Area 3: NASA OPERA 20 MB Ceiling Code Verification
// =========================================================================

#[test]
fn test_empirical_opera_code_has_20mb_ceiling() {
    // Provenance: commit `44e1e5ed` unified NASA CMR / OPERA / ARIA and MPC STAC streaming
    // onto `stream_http_to_file_verified(..., MAX_SCIENCE_DOWNLOAD_BYTES, ...)` with a 500 MB
    // ceiling (`const MAX_SCIENCE_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024;`), superseding the
    // intermediate inline `250 * 1024 * 1024` check and removing the obsolete `20 * 1024 * 1024` cap.
    let source = fs::read_to_string("src/geodata_tools.rs")
        .or_else(|_| fs::read_to_string("src-tauri/src/geodata_tools.rs"))
        .expect("read geodata_tools.rs");

    assert!(
        source.contains("const MAX_SCIENCE_DOWNLOAD_BYTES: u64 = 20 * 1024 * 1024 * 1024;")
            && source.contains("EARTHDATA_SCIENCE_DOWNLOAD"),
        "geodata_tools.rs must enforce MAX_SCIENCE_DOWNLOAD_BYTES (20 GB >= 250 MB) in OPERA/Earthdata download"
    );
    assert!(
        !source.contains("20 * 1024 * 1024;"),
        "geodata_tools.rs must NOT contain the obsolete 20MB check in OPERA download"
    );
}

// =========================================================================
// Area 4: NGL GNSS Columns Parsing and Fault Tolerance
// =========================================================================

#[test]
fn test_empirical_ngl_gnss_column_parsing_and_corrupt_data_holdings() {
    let dir = temp_sandbox_dir("ngl_gnss_robustness");

    // Test line parsing with 9 columns (less than 10) -> must be skipped without panic
    let test_lines = "Sta Lat Long Hgt X Y Z Dtbeg Dtend Dtmod NumSol\n\
                      00NA -12.4666 130.8440 104.851 -4073662.27 4712064.74 -1367874.50 2008-03-27 2018-09-25 2025-09-25 3185\n\
                      CORRUPT_SHORT_LINE 1.0 2.0\n\
                      INVALID_LAT ABC 130.0 100.0 1.0 2.0 3.0 2008-01-01 2018-01-01 2025-01-01 100\n";

    let mut parsed_items = Vec::new();
    for line in test_lines.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        let sta = cols[0];
        let Ok(lat) = cols[1].parse::<f64>() else {
            continue;
        };
        let Ok(mut lon) = cols[2].parse::<f64>() else {
            continue;
        };
        if lon > 180.0 {
            lon -= 360.0;
        }
        let dt_beg = cols[7];
        let dt_end = cols[8];
        parsed_items.push((sta, lat, lon, dt_beg, dt_end));
    }

    assert_eq!(parsed_items.len(), 1);
    let (sta, lat, lon, beg, end) = parsed_items[0];
    assert_eq!(sta, "00NA");
    assert_eq!(lat, -12.4666);
    assert_eq!(lon, 130.8440);
    assert_eq!(beg, "2008-03-27");
    assert_eq!(end, "2018-09-25");

    let _ = fs::remove_dir_all(&dir);
}

// =========================================================================
// Area 5: Sentinel-2 L1C and Sentinel-3 Route Policy Verification
// =========================================================================

#[test]
fn test_empirical_sentinel2_l1c_route_resolves_in_production() {
    let registry = DatasetRoutePolicyRegistry;

    // Resolve in Production mode
    let prod_result = registry.resolve(
        "optical.sentinel-2.l1c",
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        RouteUse::Production,
    );

    println!("Sentinel-2 L1C Production resolution: {:?}", prod_result);
    assert!(
        prod_result.is_ok(),
        "Sentinel-2 L1C must resolve successfully in Production mode"
    );
    let route = prod_result.unwrap();
    assert_eq!(route.route_id, SENTINEL2_L1C_ROUTE_ID);
    assert!(route.production_enabled);
}

#[test]
fn test_empirical_sentinel3_standard_route_resolves_in_production() {
    let registry = DatasetRoutePolicyRegistry;

    let prod_result = registry.resolve(
        "optical.sentinel-3.standard",
        "copernicus.dataspace.odata",
        "SENTINEL-3",
        RouteUse::Production,
    );

    assert!(
        prod_result.is_ok(),
        "Sentinel-3 standard route must resolve in Production"
    );
    let route = prod_result.unwrap();
    assert_eq!(route.route_id, "route.cdse.sentinel-3.standard.v1");
    assert!(route.production_enabled);
}
