//! Opt-in native tests: synthetic GeoTIFFs and the real product GIS inspector.
use super::*;
use std::fs;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

struct Case {
    root: PathBuf,
    source: PathBuf,
    output: PathBuf,
    engine: LazyNativeGisEngine,
}

impl Case {
    fn new(fixture: &str) -> Self {
        let runtime = std::env::var_os("INSAR_DEM_PASSTHROUGH_RUNTIME")
            .expect("explicit product GIS runtime required");
        let fixtures = PathBuf::from(
            std::env::var_os("INSAR_DEM_PASSTHROUGH_FIXTURES")
                .expect("explicit synthetic fixtures required"),
        );
        let base = PathBuf::from(
            std::env::var_os("INSAR_DEM_PASSTHROUGH_TEST_ROOT")
                .expect("explicit disposable test root required"),
        );
        let root = base.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT_CASE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&base).unwrap();
        fs::create_dir(&root).unwrap();
        let source = root.join("response.tif");
        fs::copy(fixtures.join(format!("{fixture}.tif")), &source).unwrap();
        Self {
            output: root.join("download.tif"),
            root,
            source,
            engine: LazyNativeGisEngine::from_product_root(runtime).unwrap(),
        }
    }

    fn prepare(&self, sources: &[PathBuf]) -> Result<serde_json::Value, String> {
        prepare_native_dem_download_output(
            &self.engine,
            "OPENTOPOGRAPHY",
            sources,
            &self.output,
            [100.0, 29.0, 101.0, 30.0],
        )
    }

    fn check_passthrough(&self) {
        let bytes = fs::read(&self.source).unwrap();
        let before = self.engine.inspect_dem(&self.source).unwrap();
        let result = self.prepare(std::slice::from_ref(&self.source)).unwrap();
        let after = self.engine.inspect_dem(&self.output).unwrap();
        assert_eq!(
            after["metadata"], before["metadata"],
            "grid, dtype and NoData must be preserved"
        );
        assert_eq!(
            fs::read(&self.output).unwrap(),
            bytes,
            "all provider file bytes must survive"
        );
        assert_eq!(result["operation"], "provider_geotiff_passthrough");
        assert_eq!(result["resampling"], serde_json::Value::Null);
        assert!(
            !self.source.exists(),
            "promotion must not leave a duplicate source TIFF"
        );
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_preserves_width_above_4096() {
    Case::new("wide").check_passthrough();
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_preserves_height_above_4096() {
    Case::new("tall").check_passthrough();
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_preserves_grid_not_aligned_with_request_bbox() {
    Case::new("unaligned").check_passthrough();
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_validated_response_replaces_an_existing_result() {
    let case = Case::new("unaligned");
    fs::write(&case.output, b"previous-result").unwrap();
    case.check_passthrough();
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_rejects_invalid_rasters_without_replacing_existing_output() {
    for fixture in ["multiband", "no-crs", "unaligned"] {
        let case = Case::new(fixture);
        if fixture == "unaligned" {
            fs::write(&case.source, b"not a GeoTIFF").unwrap();
        }
        let before = fs::read(&case.source).unwrap();
        fs::write(&case.output, b"existing-result-must-survive").unwrap();
        assert!(case.prepare(std::slice::from_ref(&case.source)).is_err());
        assert_eq!(
            fs::read(&case.output).unwrap(),
            b"existing-result-must-survive"
        );
        assert_eq!(fs::read(&case.source).unwrap(), before);
    }
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_rejects_multiple_responses_without_dropping_one() {
    let case = Case::new("unaligned");
    let second = case.root.join("second.tif");
    fs::copy(&case.source, &second).unwrap();
    assert!(case
        .prepare(&[case.source.clone(), second.clone()])
        .is_err());
    assert!(case.source.exists() && second.exists() && !case.output.exists());
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_failed_promotion_keeps_the_downloaded_response() {
    let case = Case::new("unaligned");
    fs::create_dir(&case.output).unwrap();
    let before = fs::read(&case.source).unwrap();
    assert!(case.prepare(std::slice::from_ref(&case.source)).is_err());
    assert_eq!(fs::read(&case.source).unwrap(), before);
    assert!(case.output.is_dir());
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_rejects_truncated_pixels_even_when_metadata_is_readable() {
    let case = Case::new("wide");
    let size = fs::metadata(&case.source).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&case.source)
        .unwrap()
        .set_len(size - 64)
        .unwrap();
    assert!(
        case.engine.inspect_dem(&case.source).is_ok(),
        "metadata alone is insufficient"
    );
    let truncated = fs::read(&case.source).unwrap();
    fs::write(&case.output, b"previous-result").unwrap();
    assert!(case.prepare(std::slice::from_ref(&case.source)).is_err());
    assert_eq!(fs::read(&case.output).unwrap(), b"previous-result");
    assert_eq!(fs::read(&case.source).unwrap(), truncated);
}

#[test]
#[ignore = "requires explicit product GIS runtime and synthetic fixtures"]
fn opentopography_streamed_http_response_is_promoted_byte_for_byte() {
    let case = Case::new("wide");
    let bytes = fs::read(&case.source).unwrap();
    fs::remove_file(&case.source).unwrap();
    let sent = bytes.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(stream.read(&mut request).unwrap() > 0);
        // Close-delimited response deliberately has no Content-Length.
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .unwrap();
        for chunk in sent.chunks(113) {
            stream.write_all(chunk).unwrap();
        }
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let mut events = Vec::new();
    let count = download_native_dem_geotiff(
        &case.engine,
        &client,
        None,
        &format!("http://{address}/fixture.tif"),
        &case.source,
        NativeDemTransferPolicy {
            provider: "OPENTOPOGRAPHY",
            remaining_bytes: 1024 * 1024,
        },
        &|| false,
        &mut |event| events.push(event),
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(count, bytes.len() as u64);
    assert_eq!(
        events.first(),
        Some(&insar_application_service::DemTransferEvent::BodyStarted)
    );
    assert_eq!(
        events.last(),
        Some(&insar_application_service::DemTransferEvent::BodyStopped)
    );
    case.check_passthrough();
    assert_eq!(fs::read(&case.output).unwrap(), bytes);
}
