use std::time::Instant;

use insar_core::SarRemoteSearchRequest;
use insar_provider::asf::search::AsfProvider;
use insar_provider::boundary::DataVBoundaryProvider;
use serde_json::json;

#[test]
#[ignore = "explicit live DataV + ASF ALOS PALSAR canary"]
fn hubei_real_polygon_returns_alos_palsar_metadata_footprints_and_assets() {
    let boundary_provider =
        DataVBoundaryProvider::production().expect("DataV provider initializes");
    let hubei = boundary_provider
        .resolve_exact_adcode("420000")
        .expect("Hubei exact polygon resolves");
    assert_eq!(hubei.provider_object_id, "420000");
    assert!(matches!(
        hubei.geometry_kind.as_str(),
        "polygon" | "multipolygon"
    ));

    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "provider": "asf",
            "dataset": "ALOS PALSAR",
            "platforms": ["ALOS"],
            "instrument": "PALSAR",
            "processing_levels": ["L1.1"],
            "aoi_geojson": hubei.geometry,
            "start": "2007-01-01T00:00:00Z",
            "end": "2007-12-31T23:59:59Z",
            "max_results": 3
        }
    }))
    .expect("ALOS PALSAR canary query is valid");

    let started = Instant::now();
    let response = AsfProvider::production()
        .expect("ASF provider initializes")
        .search_sar(&request, &|| false)
        .expect("real ASF ALOS PALSAR query completes");

    assert!(
        !response.products.is_empty(),
        "ALOS PALSAR canary returned no rows"
    );
    assert!(response.products.len() <= 3);
    assert!(response
        .products
        .iter()
        .all(|product| product.dataset == "ALOS PALSAR"));
    assert!(response
        .products
        .iter()
        .all(|product| product.footprint_geojson.is_some()));
    assert!(response
        .products
        .iter()
        .all(|product| !product.native_metadata.is_empty()));
    assert!(response.products.iter().any(|product| {
        product.download_url.is_some() || product.assets.iter().any(|asset| asset.url.is_some())
    }));

    eprintln!(
        "ALOS_PALSAR_CANARY elapsed_ms={} returned={} candidate_count={} mapped={} errors={}",
        started.elapsed().as_millis(),
        response.products.len(),
        response.stats.candidate_count,
        response.stats.mapped_record_count.unwrap_or(0),
        response.errors.len(),
    );
    for product in &response.products {
        eprintln!(
            "ALOS_PALSAR_PRODUCT id={} level={} footprint={} metadata_fields={} downloadable={}",
            product.product_id,
            product.processing_level.as_deref().unwrap_or("unknown"),
            product.footprint_geojson.is_some(),
            product.native_metadata.len(),
            product.download_url.is_some()
                || product.assets.iter().any(|asset| asset.url.is_some()),
        );
    }
}

#[test]
#[ignore = "explicit live ASF finite-search request-budget canary"]
fn huge_real_ers_aoi_stops_at_the_finite_working_set_budget() {
    let request: SarRemoteSearchRequest = serde_json::from_value(json!({
        "query": {
            "provider": "asf",
            "dataset": "ERS",
            "platforms": ["ERS-1"],
            "polarizations": ["VV"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[
                    [-146.97167750223, 81.18644772366277],
                    [155.265696260736, 81.18644772366277],
                    [155.265696260736, -64.86486455966102],
                    [-146.97167750223, -64.86486455966102],
                    [-146.97167750223, 81.18644772366277]
                ]]
            },
            "start": "1991-01-01T00:00:00Z",
            "end": "2011-12-31T23:59:59Z",
            "max_results": 3000
        }
    }))
    .expect("finite ERS request");

    let started = Instant::now();
    let response = AsfProvider::production()
        .expect("ASF provider initializes")
        .search_sar(&request, &|| false)
        .expect("finite ERS request completes without enumerating the remote population");

    eprintln!(
        "ERS_FINITE_BUDGET_CANARY elapsed_ms={} metadata_requests={} fetched={} mapped={} returned={} query_limit={}",
        started.elapsed().as_millis(),
        response.stats.metadata_request_count.unwrap_or(0),
        response.stats.fetched_record_count.unwrap_or(0),
        response.stats.mapped_record_count.unwrap_or(0),
        response.stats.returned_count,
        response.stats.query_limit,
    );
    assert_eq!(response.products.len(), 3_000);
    assert_eq!(response.stats.requested_limit, 3_000);
    assert!(response.stats.query_limit <= 4_000);
    assert!(response.stats.candidate_count <= 4_000);
    assert!(response.stats.fetched_record_count.unwrap_or(0) <= 4_000);
    assert!(response
        .stats
        .metadata_request_count
        .is_some_and(|count| count <= 8));
    assert_eq!(response.stats.total_count, None);
}
