//! Opt-in public metadata through the actual ApplicationService and an isolated Store.
//! No imagery, signed assets, credentials, or owner workspace is read or written.
use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationServiceError, ApplicationSetWorkingAoiRequest,
};
use insar_core::CatalogSearchQuery;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn live_freeze(sentinel2: bool, selected: bool, large_aoi: bool) {
    assert_eq!(
        std::env::var("EOSTAGE_OPTICAL_FREEZE_LIVE").as_deref(),
        Ok("1")
    );
    let root = std::env::temp_dir().join(format!(
        "sceneport-optical-freeze-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let store = root.join("catalogue.sqlite3");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&store));
    let bbox = if large_aoi {
        [70.0, 15.0, 140.0, 55.0]
    } else {
        [110.0, 30.0, 112.0, 32.0]
    };
    let geometry = json!({
        "type": "Polygon",
        "coordinates": [[
            [bbox[0], bbox[1]],
            [bbox[2], bbox[1]],
            [bbox[2], bbox[3]],
            [bbox[0], bbox[3]],
            [bbox[0], bbox[1]]
        ]]
    })
    .as_object()
    .unwrap()
    .clone();
    let aoi_name = if large_aoi {
        "Public optical large AOI"
    } else {
        "Public optical fixture"
    };
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_bbox".to_owned(),
            geometry: geometry.clone(),
            bbox: bbox.to_vec(),
            crs: "EPSG:4326".to_owned(),
            display_name: aoi_name.to_owned(),
        })
        .unwrap();
    let mut filters = if sentinel2 {
        json!({"product_type":"S2MSI2A"})
    } else {
        json!({"spacecraft":["landsat_8","landsat_9"],"processing_levels":["l2sp","l2sr"]})
    };
    if selected {
        filters["spacecraft"] = json!(["sentinel_2c"]);
        filters["datetime"] = json!({"start":"2026-05-01T00:00:00Z","end":"2026-06-01T00:00:00Z"});
    }
    let search: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": if sentinel2 { "optical.sentinel-2.l2a" } else { "optical.landsat.c2.l2" },
        "provider_id": if sentinel2 { "copernicus.dataspace.odata" } else { "microsoft.planetary-computer" },
        "native_collection_id": if sentinel2 { "SENTINEL-2" } else { "landsat-c2-l2" },
        "aoi_geojson": geometry, "cursor":null, "result_limit": if selected { 3 } else { 100 }, "filters": filters,
    })).unwrap();
    let result = service.catalog_search_and_freeze(&ApplicationCatalogSearchAndFreezeRequest {
        request_id: "public-optical-retest".to_owned(),
        search,
        batch_id: "public-optical-retest-batch".to_owned(),
        created_at: "2026-09-17T00:00:00Z".to_owned(),
        aoi_name: aoi_name.to_owned(),
        aoi_geojson: Some(geometry),
        workbench_page_size: 250,
    });
    service.shutdown().unwrap();
    drop(service);
    // Only the exact files created in this unique test directory are removed.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", store.display()));
    }
    let _ = std::fs::remove_dir(&root);
    match result {
        Ok(snapshot) => {
            let batch = snapshot.core.workbench.unwrap();
            let stats = batch
                .search_stats
                .as_ref()
                .expect("live search must freeze provenance");
            // Frozen UI stats use `truncated` for a satisfied finite selection
            // below a known remote total, not an incomplete Provider response.
            assert_eq!(
                stats.truncated,
                stats
                    .matched_count
                    .is_some_and(|total| total > stats.provider_returned_count)
            );
            assert_eq!(
                stats.provider_returned_count,
                if selected { 3 } else { 100 }
            );
            assert_eq!(stats.frozen_candidate_count as usize, batch.candidate_count);
            if sentinel2 {
                // Primary availability is not guaranteed by the public service.
                // Prove that CDSE was actually attempted, and permit only the
                // product's recorded transient fallback, never a silent skip.
                let first = stats
                    .source_attempts
                    .first()
                    .expect("CDSE attempt must be recorded");
                assert_eq!(first.provider_id, "copernicus.dataspace.odata");
                assert_eq!(first.native_collection_id, "SENTINEL-2");
                if batch.provider_id == "copernicus.dataspace.odata" {
                    assert_eq!(stats.source_attempts.len(), 1);
                    assert_eq!(first.outcome, "succeeded");
                    assert!(first.fallback_reason.is_none());
                } else {
                    assert_eq!(stats.source_attempts.len(), 2);
                    assert_eq!(first.outcome, "fallback");
                    assert!(matches!(
                        first.fallback_reason.as_deref(),
                        Some("transient_network" | "rate_limited" | "server_error")
                    ));
                    let last = &stats.source_attempts[1];
                    assert_eq!(batch.provider_id, "microsoft.planetary-computer");
                    assert_eq!(last.provider_id, batch.provider_id);
                    assert_eq!(last.native_collection_id, "sentinel-2-l2a");
                    assert_eq!(last.outcome, "succeeded");
                    assert!(last.fallback_reason.is_none());
                }
                if selected {
                    assert_eq!(
                        batch.provider_id, "copernicus.dataspace.odata",
                        "selected-platform live qualification must exercise CDSE itself"
                    );
                }
            }
            assert_eq!(batch.candidate_count, if selected { 3 } else { 100 });
            println!(
                "OPTICAL_SOURCE_ATTEMPTS {}",
                serde_json::to_string(&stats.source_attempts).unwrap()
            );
            println!(
                "OPTICAL_LIVE_FREEZE family={} selected={} provider={} count={}",
                if sentinel2 { "sentinel2" } else { "landsat" },
                selected,
                batch.provider_id,
                batch.candidate_count
            );
        }
        Err(ApplicationServiceError::Provider(error)) => panic!(
            "OPTICAL_LIVE_FREEZE failed code={} reason={}",
            error.code,
            error
                .debug_context
                .get("reason")
                .and_then(|v| v.as_str())
                .filter(|v| v.len() <= 64 && v.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
                .unwrap_or("redacted")
        ),
        Err(error) => panic!("OPTICAL_LIVE_FREEZE failed code={}", error.code()),
    }
}

#[test]
#[ignore = "public catalogue metadata; requires EOSTAGE_OPTICAL_FREEZE_LIVE=1"]
fn live_sentinel2_default_all_time_freezes_a_complete_working_set() {
    live_freeze(true, false, false);
}

#[test]
#[ignore = "public catalogue metadata; requires EOSTAGE_OPTICAL_FREEZE_LIVE=1"]
fn live_sentinel2_selected_spacecraft_freezes_through_the_application() {
    live_freeze(true, true, false);
}

#[test]
#[ignore = "public catalogue metadata; requires EOSTAGE_OPTICAL_FREEZE_LIVE=1"]
fn live_landsat_default_all_time_freezes_a_complete_working_set() {
    live_freeze(false, false, false);
}

#[test]
#[ignore = "public catalogue metadata; requires EOSTAGE_OPTICAL_FREEZE_LIVE=1"]
fn live_landsat_large_aoi_freezes_without_duplicate_identity_failure() {
    live_freeze(false, false, true);
}
