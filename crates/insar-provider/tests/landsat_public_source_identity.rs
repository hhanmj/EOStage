//! Bounded, explicit, metadata-only validation of the actual public channel.
use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{
    MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID, MPC_LANDSAT_SCENE_PROFILE_ID,
};
use insar_provider::planetary_computer::{
    validate_planetary_computer_landsat_source_id, PlanetaryComputerLandsatClient,
};
use serde_json::json;

#[test]
#[ignore = "public Microsoft catalogue; explicit EOSTAGE_LANDSAT_SOURCE_LIVE=1 required; no imagery, auth or download"]
fn live_microsoft_landsat_search_has_native_source_identity_without_download() {
    assert_eq!(
        std::env::var("EOSTAGE_LANDSAT_SOURCE_LIVE").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset":"optical.landsat.c2.l2",
        "provider_id":MPC_LANDSAT_PROVIDER_ID,
        "native_collection_id":MPC_LANDSAT_COLLECTION_ID,
        "aoi_geojson":{"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,31.0],[110.0,30.0]]]},
        "cursor":null,"result_limit":1,
        "filters":{"datetime":{"start":"2025-01-01T00:00:00Z","end":"2025-02-01T00:00:00Z"},"spacecraft":["landsat_9"],"processing_levels":["l2sp"]}
    })).unwrap();
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(response.collection.provider_id, MPC_LANDSAT_PROVIDER_ID);
    assert_eq!(
        response.collection.native_collection_id,
        MPC_LANDSAT_COLLECTION_ID
    );
    assert_eq!(
        response.collection.collection_profile_id,
        MPC_LANDSAT_SCENE_PROFILE_ID
    );
    assert_eq!(response.stats.request_count, 1);
    assert_eq!(response.products.len(), 1);
    let product = &response.products[0];
    assert_eq!(product.provider_id, MPC_LANDSAT_PROVIDER_ID);
    assert_eq!(product.properties["entity_id"], product.product_id);
    assert_eq!(
        product.properties["dataset_alias"],
        MPC_LANDSAT_COLLECTION_ID
    );
    validate_planetary_computer_landsat_source_id(&product.product_id, &product.display_name)
        .unwrap();
    assert!(product.footprint_geojson.is_some() && product.acquisition_datetime.is_some());
    assert!(product.assets.is_empty() && product.payload.is_empty());
    // Only public identities, not response payloads, URLs or signed material.
    println!(
        "{}",
        json!({"provider":product.provider_id,"collection":response.collection.native_collection_id,
        "native_id":product.product_id,"scene_id":product.properties["landsat_scene_id"],
        "time":product.acquisition_datetime,"requests":response.stats.request_count,
        "matched_count":response.stats.matched_count,"remote_exhausted":response.stats.remote_exhausted,
        "returned":response.stats.returned_count,"imagery_downloaded":false})
    );
}

#[test]
#[ignore = "public Microsoft Landsat default working set; explicit EOSTAGE_LANDSAT_SOURCE_LIVE=1 required; metadata only"]
fn live_microsoft_landsat_default_working_set_respects_requested_limit() {
    assert_eq!(
        std::env::var("EOSTAGE_LANDSAT_SOURCE_LIVE").as_deref(),
        Ok("1")
    );
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset":"optical.landsat.c2.l2",
        "provider_id":MPC_LANDSAT_PROVIDER_ID,
        "native_collection_id":MPC_LANDSAT_COLLECTION_ID,
        "aoi_geojson":{"type":"Polygon","coordinates":[[
            [110.0,30.0], [112.0,30.0], [112.0,32.0],
            [110.0,32.0], [110.0,30.0]
        ]]},
        "cursor":null,"result_limit":100,
        "filters":{"spacecraft":["landsat_8","landsat_9"],"processing_levels":["l2sp","l2sr"]}
    }))
    .unwrap();
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::direct()).unwrap();
    let response = client.search_catalog(&query, &|| false).unwrap();
    response.validate().unwrap();
    assert_eq!(response.products.len(), 100);
    assert_eq!(response.stats.requested_limit, Some(100));
    assert_eq!(response.stats.returned_count, 100);
    assert!(response.stats.working_set_satisfied && !response.stats.truncated);
}
