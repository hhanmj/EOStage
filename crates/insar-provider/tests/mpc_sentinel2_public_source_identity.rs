//! Bounded metadata-only validation of the reviewed Microsoft Sentinel-2 search mapper.
use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{
    MpcSentinel2CatalogClient, MPC_SENTINEL2_L2A_COLLECTION_ID, MPC_SENTINEL2_L2A_PROFILE_ID,
    MPC_SENTINEL2_L2A_PROVIDER_ID,
};
use serde_json::json;

#[test]
#[ignore = "fixed public Microsoft Sentinel-2 metadata; explicit EOSTAGE_STAC_PUBLIC_VALIDATION=1 required; no imagery, auth or download"]
fn live_microsoft_sentinel2_search_preserves_native_identity_and_stays_search_only() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":MPC_SENTINEL2_L2A_PROVIDER_ID,
        "native_collection_id":MPC_SENTINEL2_L2A_COLLECTION_ID,
        "aoi_geojson":{"type":"Polygon","coordinates":[[
            [121.4,25.0],[121.6,25.0],[121.6,25.2],[121.4,25.2],[121.4,25.0]]]},
        "cursor":null,"result_limit":1,
        "filters":{"product_type":"S2MSI2A",
            "datetime":{"start":"2025-01-01T00:00:00Z","end":"2025-01-31T23:59:59Z"},
            "cloud_cover_lte":100.0}
    }))
    .unwrap();
    let client = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::system()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(
        response.collection.provider_id,
        MPC_SENTINEL2_L2A_PROVIDER_ID
    );
    assert_eq!(
        response.collection.native_collection_id,
        MPC_SENTINEL2_L2A_COLLECTION_ID
    );
    assert_eq!(
        response.collection.collection_profile_id,
        MPC_SENTINEL2_L2A_PROFILE_ID
    );
    assert!(response.collection.production_enabled);
    assert_eq!(response.products.len(), 1);
    let product = &response.products[0];
    assert_eq!(product.provider_id, MPC_SENTINEL2_L2A_PROVIDER_ID);
    assert!(product.product_id.starts_with("S2"));
    assert!(product.display_name.ends_with(".SAFE"));
    assert_eq!(product.properties["product_type"], "S2MSI2A");
    assert!(product.properties["cloud_cover"].as_f64().is_some());
    assert!(product.acquisition_datetime.is_some() && product.footprint_geojson.is_some());
    assert!(product.assets.is_empty() && product.payload.is_empty());
    assert_eq!(response.stats.request_count, 1);
    assert!(response.stats.working_set_satisfied && !response.stats.truncated);
    println!(
        "MPC_SENTINEL2_PRODUCTION_SUMMARY {}",
        json!({
            "provider":product.provider_id,
            "collection":response.collection.native_collection_id,
            "native_item_id":product.product_id,
            "safe_product":product.display_name,
            "datetime":product.acquisition_datetime,
            "cloud_cover":product.properties["cloud_cover"],
            "requests":response.stats.request_count,
            "assets":product.assets.len(),
            "preview_admitted":false,
            "download_admitted":false,
            "image_requests":0
        })
    );
}

#[test]
#[ignore = "public Microsoft Sentinel-2 default working set; explicit EOSTAGE_STAC_PUBLIC_VALIDATION=1 required; metadata only"]
fn live_microsoft_sentinel2_default_working_set_accepts_current_public_items() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":MPC_SENTINEL2_L2A_PROVIDER_ID,
        "native_collection_id":MPC_SENTINEL2_L2A_COLLECTION_ID,
        "aoi_geojson":{"type":"Polygon","coordinates":[[
            [110.0,30.0], [112.0,30.0], [112.0,32.0],
            [110.0,32.0], [110.0,30.0]
        ]]},
        "cursor":null,"result_limit":100,
        "filters":{"product_type":"S2MSI2A","cloud_cover_lte":100.0}
    }))
    .unwrap();
    let client = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::system()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(response.products.len(), 100);
    assert_eq!(response.stats.returned_count, 100);
    assert!(response.stats.working_set_satisfied && !response.stats.truncated);
    assert!(response.products.iter().all(|product| {
        product.provider_id == MPC_SENTINEL2_L2A_PROVIDER_ID
            && product.assets.is_empty()
            && product.payload.is_empty()
    }));
}

#[test]
#[ignore = "public Microsoft Sentinel-2 blank-cloud working set; explicit EOSTAGE_STAC_PUBLIC_VALIDATION=1 required; metadata only"]
fn live_microsoft_sentinel2_blank_cloud_filter_accepts_current_public_items() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset":"optical.sentinel-2.l2a",
        "provider_id":MPC_SENTINEL2_L2A_PROVIDER_ID,
        "native_collection_id":MPC_SENTINEL2_L2A_COLLECTION_ID,
        "aoi_geojson":{"type":"Polygon","coordinates":[[
            [110.0,30.0], [112.0,30.0], [112.0,32.0],
            [110.0,32.0], [110.0,30.0]
        ]]},
        "cursor":null,"result_limit":100,
        "filters":{"product_type":"S2MSI2A"}
    }))
    .unwrap();
    let client = MpcSentinel2CatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(response.products.len(), 100);
}
