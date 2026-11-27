//! Issue #3 R2 B5: opt-in live probe of the Microsoft HLS adapter alone.
use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{MpcHlsCatalogClient, MpcHlsProduct, MPC_HLS_PROVIDER_ID};
use serde_json::json;

#[test]
#[ignore = "requires explicit opt-in and public Microsoft Planetary Computer access"]
fn hls_adapter_live_probe() {
    // Direct: this host's system proxy setting is unrelated to the adapter.
    let client = MpcHlsCatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
    for (product, platforms) in [
        (MpcHlsProduct::L30, json!(["landsat_8", "landsat_9"])),
        (MpcHlsProduct::S30, json!(["sentinel_2a", "sentinel_2b"])),
    ] {
        let query = CatalogSearchQuery {
            logical_dataset: product.logical_dataset().to_owned(),
            provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
            native_collection_id: product.collection_id().to_owned(),
            aoi_geojson: Some(
                json!({"type":"Polygon","coordinates":[[[110.0,30.0],[110.5,30.0],[110.5,30.5],[110.0,30.5],[110.0,30.0]]]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            cursor: None,
            result_limit: Some(10),
            filters: json!({
                "spacecraft": platforms,
                "cloud_cover_lte": 80.0,
                "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        match client.search_catalog(&query, &|| false) {
            Ok(response) => {
                eprintln!(
                    "HLS_PROBE {:?} products={} stats={}",
                    product,
                    response.products.len(),
                    serde_json::to_string(&response.stats).unwrap()
                );
                for p in &response.products {
                    eprintln!(
                        "HLS_PROBE item {} {}",
                        p.product_id,
                        serde_json::to_string(&p.properties).unwrap()
                    );
                }
            }
            Err(error) => panic!("HLS_PROBE {:?} error {:?}", product, error),
        }
    }
}
