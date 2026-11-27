//! Explicit, fixed metadata-only acceptance of the production search mapper.
use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{
    EarthSearchLandsatCatalogClient, EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    EARTH_SEARCH_LANDSAT_PROVIDER_ID, EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET,
};
use insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id;
use serde_json::json;

#[test]
#[ignore = "fixed public Earth Search metadata; explicit EOSTAGE_STAC_PUBLIC_VALIDATION=1 required; no assets, credentials or downloads"]
fn production_earth_search_preserves_science_and_search_only_identity() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": LANDSAT_C2_L2_LOGICAL_DATASET,
        "provider_id": EARTH_SEARCH_LANDSAT_PROVIDER_ID,
        "native_collection_id": EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        "aoi_geojson": {"type":"Polygon","coordinates":[[
            [111.0,31.5],[111.2,31.5],[111.2,31.7],[111.0,31.7],[111.0,31.5]]]},
        "result_limit":1,
        "filters":{"datetime":{"start":"2025-01-11T00:00:00Z","end":"2025-01-11T23:59:59Z"},
            "spacecraft":["landsat_9"],"processing_levels":["l2sp"],"cloud_cover_lte":100.0}
    }))
    .unwrap();
    let client = EarthSearchLandsatCatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(
        response.collection.provider_id,
        EARTH_SEARCH_LANDSAT_PROVIDER_ID
    );
    assert_eq!(
        response.collection.collection_profile_id,
        EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID
    );
    assert!(response.collection.production_enabled);
    assert_eq!(response.products.len(), 1);
    let product = &response.products[0];
    assert_eq!(product.provider_id, EARTH_SEARCH_LANDSAT_PROVIDER_ID);
    assert_eq!(product.product_id, "LC09_L2SP_125038_20250111_02_T1");
    validate_planetary_computer_landsat_source_id(&product.product_id, &product.display_name)
        .unwrap();
    assert_eq!(product.properties["entity_id"], product.product_id);
    assert_eq!(
        product.properties["landsat_scene_id"],
        "LC91250382025011LGN00"
    );
    assert_eq!(product.properties["spacecraft"], "landsat_9");
    assert_eq!(product.properties["processing_level"], "l2sp");
    assert_eq!(product.properties["wrs_path"], "125");
    assert_eq!(product.properties["wrs_row"], "038");
    assert_eq!(product.properties["browse_available"], false);
    assert!(product.assets.is_empty() && product.payload.is_empty());
    assert!(product.footprint_geojson.is_some());
    assert_eq!(response.stats.request_count, 1);
    assert!(response.stats.working_set_satisfied && !response.stats.truncated);
    assert!(response.next_cursor.is_none() && response.item_errors.is_empty());
    println!(
        "EARTH_SEARCH_PRODUCTION_SUMMARY {}",
        json!({
            "provider":product.provider_id,"item_id":product.product_id,"display_id":product.display_name,
            "datetime":product.acquisition_datetime,"requests":response.stats.request_count,
            "remote_total":response.stats.matched_count,"assets":product.assets.len(),
            "preview_admitted":false,"download_admitted":false,"image_requests":0
        })
    );
}
