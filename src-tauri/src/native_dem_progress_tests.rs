use super::*;
use insar_application_service::DemTransferEvent;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn native_dem_http_body_reports_activity_before_raster_validation() {
    exercise_http_body(false);
}

#[test]
fn native_dem_http_body_stops_activity_on_truncated_response() {
    exercise_http_body(true);
}

#[test]
fn online_dem_vertical_datum_follows_dataset_truth_and_rejects_conflicts() {
    for dataset in ["COP30", "COP90"] {
        assert_eq!(
            native_dem_source_vertical_datum(&serde_json::json!({ "dataset": dataset })).unwrap(),
            "EGM2008"
        );
        assert_eq!(
            native_dem_source_vertical_datum(&serde_json::json!({
                "dataset": dataset,
                "source_vertical_datum": "auto"
            }))
            .unwrap(),
            "EGM2008"
        );
        assert!(native_dem_source_vertical_datum(&serde_json::json!({
            "dataset": dataset,
            "source_vertical_datum": "EGM96"
        }))
        .unwrap_err()
        .starts_with("NATIVE_GIS_VERTICAL_DATUM_MISMATCH:"));
    }
    assert_eq!(
        native_dem_source_vertical_datum(&serde_json::json!({ "dataset": "NASADEM" })).unwrap(),
        "EGM96"
    );
    assert_eq!(
        native_dem_source_vertical_datum(&serde_json::json!({
            "dataset": "SRTM_GL1_ELLIPSOIDAL"
        }))
        .unwrap(),
        "WGS84_ELLIPSOID"
    );
}

fn exercise_http_body(truncated: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0; 4096];
        assert!(stream.read(&mut buffer).unwrap() > 0, "request must arrive");
        // Header-shaped fixture, deliberately not an accepted raster. The test
        // covers real HTTP body activity and stop-before-validation, not GIS acceptance.
        let declared_length = if truncated { 16 } else { 8 };
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.write_all(b"II*\0\0\0\0\0").unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "insar-dem-body-{}-{}",
        std::process::id(),
        address.port()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut events = Vec::new();
    let result = download_native_dem_geotiff(
        &LazyNativeGisEngine::unavailable(),
        &reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        None,
        &format!("http://{address}/fixture.tif"),
        &root.join("tile.tif"),
        NativeDemTransferPolicy {
            provider: "COPERNICUS_AWS",
            remaining_bytes: 1024,
        },
        &|| false,
        &mut |event| events.push(event),
    );
    server.join().unwrap();
    assert!(
        result.is_err(),
        "the fixture must not count as an accepted raster"
    );
    assert_eq!(events.first(), Some(&DemTransferEvent::BodyStarted));
    assert_eq!(events.last(), Some(&DemTransferEvent::BodyStopped));
    assert!(events.contains(&DemTransferEvent::BytesReceived(8)));
    if truncated {
        assert!(result
            .unwrap_err()
            .starts_with("NATIVE_DEM_NETWORK_FAILED:"));
    } else {
        assert!(result
            .unwrap_err()
            .starts_with("NATIVE_DEM_INVALID_RASTER:"));
    }
    assert!(!root.join("tile.tif").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn aws_pause_preserves_a_validated_part_and_resumes_with_range_and_if_range() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        first
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(first.read(&mut request).unwrap() > 0);
        first
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\nII*\0AAAA",
            )
            .unwrap();
        drop(first);

        let (mut second, _) = listener.accept().unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let count = second.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
        assert!(request.contains("range: bytes=8-"));
        assert!(request.contains("if-range: \"fixture-v1\""));
        second
            .write_all(
                b"HTTP/1.1 206 Partial Content\r\nContent-Length: 8\r\nContent-Range: bytes 8-15/16\r\nETag: \"fixture-v1\"\r\nConnection: close\r\n\r\nBBBBBBBB",
            )
            .unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "insar-dem-resume-{}-{}",
        std::process::id(),
        address.port()
    ));
    std::fs::create_dir(&root).unwrap();
    let destination = root.join("tile.tif");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let paused = AtomicBool::new(false);
    let first = download_native_dem_geotiff(
        &LazyNativeGisEngine::unavailable(),
        &client,
        None,
        &format!("http://{address}/fixture.tif"),
        &destination,
        NativeDemTransferPolicy {
            provider: "COPERNICUS_AWS",
            remaining_bytes: 1024,
        },
        &|| paused.load(Ordering::Acquire),
        &mut |event| {
            if matches!(event, DemTransferEvent::BytesReceived(8)) {
                paused.store(true, Ordering::Release);
            }
        },
    );
    assert!(first.unwrap_err().starts_with("NATIVE_DEM_INTERRUPTED:"));
    let temporary = destination.with_file_name(".tile.tif.download.part");
    assert_eq!(std::fs::metadata(&temporary).unwrap().len(), 8);
    assert!(native_dem_resume_metadata_path(&temporary).is_file());

    paused.store(false, Ordering::Release);
    let resumed = download_native_dem_geotiff(
        &LazyNativeGisEngine::unavailable(),
        &client,
        None,
        &format!("http://{address}/fixture.tif"),
        &destination,
        NativeDemTransferPolicy {
            provider: "COPERNICUS_AWS",
            remaining_bytes: 1024,
        },
        &|| false,
        &mut |_| {},
    );
    assert!(
        resumed
            .unwrap_err()
            .starts_with("NATIVE_DEM_INVALID_RASTER:"),
        "synthetic truncated TIFF must fail structural validation even without native GIS"
    );
    assert!(!destination.exists());
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn opentopography_pause_discards_non_resumable_partial_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\nII*\0AAAA",
            )
            .unwrap();
    });
    let root = std::env::temp_dir().join(format!(
        "insar-dem-opentopo-pause-{}-{}",
        std::process::id(),
        address.port()
    ));
    std::fs::create_dir(&root).unwrap();
    let destination = root.join("tile.tif");
    let paused = AtomicBool::new(false);
    let result = download_native_dem_geotiff(
        &LazyNativeGisEngine::unavailable(),
        &reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        None,
        &format!("http://{address}/fixture.tif"),
        &destination,
        NativeDemTransferPolicy {
            provider: "OPENTOPOGRAPHY",
            remaining_bytes: 1024,
        },
        &|| paused.load(Ordering::Acquire),
        &mut |event| {
            if matches!(event, DemTransferEvent::BytesReceived(8)) {
                paused.store(true, Ordering::Release);
            }
        },
    );
    assert!(result.unwrap_err().starts_with("NATIVE_DEM_INTERRUPTED:"));
    assert!(!destination
        .with_file_name(".tile.tif.download.part")
        .exists());
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
