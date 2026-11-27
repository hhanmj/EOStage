use insar_core::{CatalogSearchQuery, FrozenSearchBatch};
use serde_json::json;

#[test]
fn catalogue_query_accepts_an_explicit_no_aoi_request() {
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": null,
        "cursor": null,
        "result_limit": 24,
        "filters": {}
    }))
    .expect("no-AOI catalogue query is part of the Golden contract");

    assert!(query.aoi_geojson.is_none());
}

#[test]
fn frozen_search_batch_preserves_no_aoi_without_a_world_bbox() {
    let batch: FrozenSearchBatch = serde_json::from_value(json!({
        "schema_version": 1,
        "batch_id": "golden-no-aoi-batch",
        "provider_id": "asf.catalog",
        "created_at": "2026-08-15T00:00:00Z",
        "aoi_name": "未限定 AOI",
        "aoi_geojson": null,
        "query": {"aoi_geojson": null},
        "candidates": []
    }))
    .expect("no-AOI frozen batch is readable");

    assert!(batch.aoi_geojson.is_none());
    assert!(batch.validate().is_ok());
}
