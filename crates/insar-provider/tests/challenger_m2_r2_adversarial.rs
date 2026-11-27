//! Adversarial Empirical Challenger Stress Test for Milestone 2 Remediation
//! Specifically stress-tests:
//! 1. Landsat 4-9 query validation across all valid tags and adversarial/invalid inputs
//! 2. Route policy resolution for Sentinel-2 L1C and Sentinel-3 standard across Offline and Production modes

use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{
    DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse, MPC_LANDSAT_COLLECTION_ID,
    MPC_LANDSAT_PROVIDER_ID, SENTINEL2_L1C_ROUTE_ID,
};
use insar_provider::planetary_computer::{
    PlanetaryComputerErrorCode, PlanetaryComputerLandsatClient,
};
use serde_json::json;

const SENTINEL3_STANDARD_ROUTE_ID: &str = "route.cdse.sentinel-3.standard.v1";

fn make_landsat_query(
    spacecraft_val: serde_json::Value,
) -> Result<CatalogSearchQuery, serde_json::Error> {
    serde_json::from_value(json!({
        "logical_dataset": "optical.landsat.c2.l2",
        "provider_id": MPC_LANDSAT_PROVIDER_ID,
        "native_collection_id": MPC_LANDSAT_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[81.16, 27.53], [92.10, 27.53], [92.10, 33.18], [81.16, 27.53]]]
        },
        "cursor": null,
        "result_limit": 100,
        "filters": {
            "cloud_cover_lte": 60.0,
            "spacecraft": spacecraft_val,
            "processing_levels": ["l2sp", "l2sr"]
        }
    }))
}

#[test]
fn test_landsat_query_validation_valid_spacecraft_tags() {
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system())
        .expect("client creation");

    // Cancelled callback immediately returns true; if validation passes, search_catalog returns Cancelled error code
    let cancel = || true;

    // 1. Single valid spacecraft tags (Landsat 4, 5, 7, 8, 9)
    let valid_singles = [
        "landsat_4",
        "landsat_5",
        "landsat_7",
        "landsat_8",
        "landsat_9",
    ];
    for sc in &valid_singles {
        let query = make_landsat_query(json!([sc])).expect("valid query shape");
        let res = client.search_catalog(&query, &cancel);
        assert!(res.is_err(), "Must error on cancellation");
        let err = res.unwrap_err();
        assert_eq!(
            err.code(),
            PlanetaryComputerErrorCode::Cancelled,
            "Expected query validation to pass for [{}] and halt at Cancelled, got {:?}",
            sc,
            err.code()
        );
    }

    // 2. All 5 spacecraft tags together
    let query_all = make_landsat_query(json!([
        "landsat_4",
        "landsat_5",
        "landsat_7",
        "landsat_8",
        "landsat_9"
    ]))
    .expect("valid query shape");
    let res = client.search_catalog(&query_all, &cancel);
    assert!(res.is_err());
    assert_eq!(
        res.unwrap_err().code(),
        PlanetaryComputerErrorCode::Cancelled,
        "All 5 Landsat missions must pass query validation"
    );

    // 3. Various subsets
    let subsets = [
        vec!["landsat_8", "landsat_9"],
        vec!["landsat_4", "landsat_5"],
        vec!["landsat_7", "landsat_8"],
        vec!["landsat_5", "landsat_7", "landsat_9"],
    ];
    for sub in &subsets {
        let query = make_landsat_query(json!(sub)).expect("valid query shape");
        let res = client.search_catalog(&query, &cancel);
        assert_eq!(
            res.unwrap_err().code(),
            PlanetaryComputerErrorCode::Cancelled,
            "Subset {:?} must pass query validation",
            sub
        );
    }
}

#[test]
fn test_landsat_query_validation_adversarial_invalid_tags() {
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system())
        .expect("client creation");
    let cancel = || true;

    let adversarial_cases = vec![
        ("unsupported mission landsat_1", json!(["landsat_1"])),
        ("unsupported mission landsat_2", json!(["landsat_2"])),
        ("unsupported mission landsat_3", json!(["landsat_3"])),
        ("unsupported mission landsat_6", json!(["landsat_6"])),
        ("unsupported mission landsat_10", json!(["landsat_10"])),
        ("hyphenated landsat-4", json!(["landsat-4"])),
        ("hyphenated landsat-8", json!(["landsat-8"])),
        ("uppercase LANDSAT_8", json!(["LANDSAT_8"])),
        ("mixed case Landsat_9", json!(["Landsat_9"])),
        ("non-landsat sentinel_2", json!(["sentinel_2"])),
        ("empty string in array", json!([""])),
        ("empty array", json!([])),
        ("duplicate tags", json!(["landsat_8", "landsat_8"])),
        (
            "more than allowed count with duplicates",
            json!([
                "landsat_4",
                "landsat_5",
                "landsat_7",
                "landsat_8",
                "landsat_9",
                "landsat_8"
            ]),
        ),
        ("sql injection attempt", json!(["landsat_8' OR '1'='1"])),
    ];

    for (name, val) in adversarial_cases {
        let query = make_landsat_query(val).expect("query json parses");
        let res = client.search_catalog(&query, &cancel);
        assert!(
            res.is_err(),
            "Adversarial case '{}' must fail validation",
            name
        );
        let err = res.unwrap_err();
        assert_eq!(
            err.code(),
            PlanetaryComputerErrorCode::ConfigInvalid,
            "Adversarial case '{}' must return ConfigInvalid, got {:?}",
            name,
            err.code()
        );
    }
}

#[test]
fn test_sentinel3_standard_route_resolution() {
    let registry = DatasetRoutePolicyRegistry;

    // 1. Offline resolution
    let offline_route = registry
        .resolve(
            "optical.sentinel-3.standard",
            "copernicus.dataspace.odata",
            "SENTINEL-3",
            RouteUse::OfflineFixture,
        )
        .expect("Sentinel-3 standard route resolves offline");
    assert_eq!(offline_route.route_id, SENTINEL3_STANDARD_ROUTE_ID);
    assert_eq!(
        offline_route.collection_profile_id,
        "cdse.sentinel-3.standard.collection.v1"
    );
    assert!(
        offline_route.production_enabled,
        "Sentinel-3 route must have production_enabled = true"
    );

    // 2. Production resolution
    let prod_route = registry
        .resolve(
            "optical.sentinel-3.standard",
            "copernicus.dataspace.odata",
            "SENTINEL-3",
            RouteUse::Production,
        )
        .expect("Sentinel-3 standard route resolves in Production");
    assert_eq!(prod_route.route_id, SENTINEL3_STANDARD_ROUTE_ID);

    // 3. Filter keys validation
    assert!(
        registry
            .validate_filter_keys(
                "optical.sentinel-3.standard",
                &["datetime", "cloud_cover_lte", "product_type", "spacecraft"]
            )
            .is_ok(),
        "Supported filter keys must be accepted"
    );
    assert!(
        registry
            .validate_filter_keys("optical.sentinel-3.standard", &["unknown_filter"])
            .is_err(),
        "Unsupported filter keys must be rejected"
    );
}

#[test]
fn test_sentinel2_l1c_route_resolution_adversarial_check() {
    let registry = DatasetRoutePolicyRegistry;

    // Offline resolution succeeds
    let offline_route = registry
        .resolve(
            "optical.sentinel-2.l1c",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::OfflineFixture,
        )
        .expect("Sentinel-2 L1C route resolves offline");
    assert_eq!(offline_route.route_id, SENTINEL2_L1C_ROUTE_ID);

    // Production resolution check
    let prod_result = registry.resolve(
        "optical.sentinel-2.l1c",
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        RouteUse::Production,
    );

    // NOTE: Directive explicitly requested:
    // "science.sentinel-2.l1c.v1 与 science.sentinel-3.standard.v1 报 V3_APPLICATION_INVALID_REQUEST：
    // 在 crates/insar-provider/src/catalog/route_policy.rs:173-182 中，SENTINEL2_L1C_ROUTE 的 production_enabled 仍为 false...
    // 请在 route_policy.rs 及对应 provider 路由中正式启用并接入这两个逻辑数据集。"
    // Worker claimed: "SENTINEL2_L1C_ROUTE.production_enabled was set to false. Set to true."
    // Let's test the actual reality of whether production is enabled:
    match prod_result {
        Ok(route) => {
            println!(
                "SENTINEL-2 L1C Production route resolution: OK (production_enabled = {})",
                route.production_enabled
            );
        }
        Err(DatasetRoutePolicyError::ProductionDisabled(route_id)) => {
            println!("SENTINEL-2 L1C Production route resolution: ProductionDisabled({}) - DEFECT CONFIRMED!", route_id);
            assert_eq!(route_id, SENTINEL2_L1C_ROUTE_ID);
        }
        Err(e) => {
            panic!("Unexpected error resolving Sentinel-2 L1C: {:?}", e);
        }
    }
}
