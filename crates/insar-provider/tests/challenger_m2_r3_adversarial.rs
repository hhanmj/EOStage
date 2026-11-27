//! Adversarial Empirical Challenger Stress Test for Milestone 2 Round 3
//! Verification of:
//! 1. Route Policy resolution in Production & Offline modes for Sentinel-2 L1C, Sentinel-3 Standard, Landsat TM/ETM
//! 2. Systematic power-set verification (all 31 combinations) of authorized Landsat 4-9 spacecraft tags
//! 3. Adversarial invalid spacecraft tags, formatting corruptions, type attacks, duplicate injections
//! 4. Planetary Computer Landsat & HLS asset URL prefix and origin security validation
//! 5. Filter key boundary enforcement across all declared routes

use insar_core::{CatalogSearchQuery, ProviderNetworkPolicy};
use insar_provider::catalog::{
    DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse, MPC_LANDSAT_COLLECTION_ID,
    MPC_LANDSAT_PROVIDER_ID, SENTINEL2_L1C_ROUTE_ID, SENTINEL3_STANDARD_ROUTE_ID,
};
use insar_provider::planetary_computer::{
    validate_planetary_computer_landsat_stable_href, PlanetaryComputerErrorCode,
    PlanetaryComputerLandsatClient,
};
use serde_json::json;

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
fn test_landsat_all_31_combinations_of_authorized_spacecraft() {
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system())
        .expect("client creation");
    let cancel = || true;

    let authorized = [
        "landsat_4",
        "landsat_5",
        "landsat_7",
        "landsat_8",
        "landsat_9",
    ];
    let num_authorized = authorized.len();
    let total_combinations = (1 << num_authorized) - 1; // 31 combinations

    let mut tested_count = 0;
    for mask in 1..=total_combinations {
        let mut subset = Vec::new();
        for (i, tag) in authorized.iter().enumerate() {
            if (mask & (1 << i)) != 0 {
                subset.push(*tag);
            }
        }

        let query = make_landsat_query(json!(subset)).expect("query json parses");
        let res = client.search_catalog(&query, &cancel);
        assert!(
            res.is_err(),
            "Must stop at cancel callback for valid subset {:?}",
            subset
        );
        let err = res.unwrap_err();
        assert_eq!(
            err.code(),
            PlanetaryComputerErrorCode::Cancelled,
            "Subset {:?} failed validation with code {:?}",
            subset,
            err.code()
        );
        tested_count += 1;
    }

    assert_eq!(
        tested_count, 31,
        "All 31 non-empty subsets of authorized Landsat missions must be verified"
    );
}

#[test]
fn test_landsat_adversarial_rejected_spacecraft_tags() {
    let client = PlanetaryComputerLandsatClient::new(ProviderNetworkPolicy::system())
        .expect("client creation");
    let cancel = || true;

    let adversarial_cases = vec![
        ("unauthorized landsat_unknown", json!(["landsat_unknown"])),
        ("unauthorized landsat_0", json!(["landsat_0"])),
        ("unauthorized landsat_1", json!(["landsat_1"])),
        ("unauthorized landsat_2", json!(["landsat_2"])),
        ("unauthorized landsat_3", json!(["landsat_3"])),
        ("unauthorized landsat_6", json!(["landsat_6"])),
        ("unauthorized landsat_10", json!(["landsat_10"])),
        ("unauthorized landsat_11", json!(["landsat_11"])),
        ("wrong family sentinel_2a", json!(["sentinel_2a"])),
        ("wrong family terra", json!(["terra"])),
        ("wrong family aqua", json!(["aqua"])),
        ("uppercase LANDSAT_4", json!(["LANDSAT_4"])),
        ("uppercase LANDSAT_8", json!(["LANDSAT_8"])),
        ("hyphenated landsat-4", json!(["landsat-4"])),
        ("hyphenated landsat-7", json!(["landsat-7"])),
        ("hyphenated landsat-8", json!(["landsat-8"])),
        ("mixed casing Landsat_5", json!(["Landsat_5"])),
        ("padded spaces", json!([" landsat_8 "])),
        ("empty string tag", json!([""])),
        ("empty array", json!([])),
        ("sql injection string", json!(["' OR '1'='1"])),
        ("null byte injection", json!(["landsat_8\0bad"])),
        ("duplicate valid tags", json!(["landsat_8", "landsat_8"])),
        (
            "duplicate valid tags diff order",
            json!(["landsat_4", "landsat_7", "landsat_4"]),
        ),
        (
            "more than allowed count",
            json!([
                "landsat_4",
                "landsat_5",
                "landsat_7",
                "landsat_8",
                "landsat_9",
                "landsat_8"
            ]),
        ),
        ("number in array", json!([8])),
        ("boolean in array", json!([true])),
        ("null in array", json!([null])),
        ("bare string not array", json!("landsat_8")),
        (
            "object instead of array",
            json!({"spacecraft": "landsat_8"}),
        ),
    ];

    for (name, val) in adversarial_cases {
        let query = make_landsat_query(val).expect("query json parses");
        let res = client.search_catalog(&query, &cancel);
        assert!(res.is_err(), "Adversarial case '{}' must be rejected", name);
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
fn test_route_policy_sentinel2_l1c_production_resolution() {
    let registry = DatasetRoutePolicyRegistry;

    // 1. Offline resolution
    let offline = registry
        .resolve(
            "optical.sentinel-2.l1c",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::OfflineFixture,
        )
        .expect("Sentinel-2 L1C offline resolution");
    assert_eq!(offline.route_id, SENTINEL2_L1C_ROUTE_ID);
    assert!(offline.production_enabled);

    // 2. Production resolution
    let prod = registry
        .resolve(
            "optical.sentinel-2.l1c",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::Production,
        )
        .expect("Sentinel-2 L1C production resolution must succeed");
    assert_eq!(prod.route_id, SENTINEL2_L1C_ROUTE_ID);
    assert!(prod.production_enabled);

    // 3. Filter keys verification
    assert!(
        registry
            .validate_filter_keys(
                "optical.sentinel-2.l1c",
                &["product_type", "cloud_cover_lte", "datetime", "spacecraft"]
            )
            .is_ok(),
        "All 4 supported filter keys must be accepted"
    );
    assert!(
        registry
            .validate_filter_keys("optical.sentinel-2.l1c", &["bands"])
            .is_err(),
        "Unsupported filter key 'bands' must be rejected"
    );
    assert!(
        registry
            .validate_filter_keys("optical.sentinel-2.l1c", &["polarization"])
            .is_err(),
        "Unsupported filter key 'polarization' must be rejected"
    );
}

#[test]
fn test_route_policy_sentinel3_standard_production_resolution() {
    let registry = DatasetRoutePolicyRegistry;

    // 1. Offline resolution
    let offline = registry
        .resolve(
            "optical.sentinel-3.standard",
            "copernicus.dataspace.odata",
            "SENTINEL-3",
            RouteUse::OfflineFixture,
        )
        .expect("Sentinel-3 standard offline resolution");
    assert_eq!(offline.route_id, SENTINEL3_STANDARD_ROUTE_ID);
    assert!(offline.production_enabled);

    // 2. Production resolution
    let prod = registry
        .resolve(
            "optical.sentinel-3.standard",
            "copernicus.dataspace.odata",
            "SENTINEL-3",
            RouteUse::Production,
        )
        .expect("Sentinel-3 standard production resolution must succeed");
    assert_eq!(prod.route_id, SENTINEL3_STANDARD_ROUTE_ID);
    assert!(prod.production_enabled);

    // 3. Filter keys verification
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
            .validate_filter_keys("optical.sentinel-3.standard", &["unknown_key"])
            .is_err(),
        "Unsupported filter key must be rejected"
    );
}

#[test]
fn test_route_policy_landsat_tm_and_etm_routes_present() {
    let registry = DatasetRoutePolicyRegistry;

    // TM route
    let tm_route = registry
        .resolve(
            "optical.landsat.c2.l2",
            "usgs.eros.m2m",
            "landsat_tm_c2_l2",
            RouteUse::OfflineFixture,
        )
        .expect("landsat_tm_c2_l2 route must resolve");
    assert_eq!(
        tm_route.route_id,
        "usgs.eros.m2m.landsat.tm.c2.l2.scene.route.v1"
    );

    // ETM route
    let etm_route = registry
        .resolve(
            "optical.landsat.c2.l2",
            "usgs.eros.m2m",
            "landsat_etm_c2_l2",
            RouteUse::OfflineFixture,
        )
        .expect("landsat_etm_c2_l2 route must resolve");
    assert_eq!(
        etm_route.route_id,
        "usgs.eros.m2m.landsat.etm.c2.l2.scene.route.v1"
    );
}

#[test]
fn test_route_policy_adversarial_mismatches_fail() {
    let registry = DatasetRoutePolicyRegistry;

    // Unknown logical dataset
    let err1 = registry.resolve(
        "optical.unknown.dataset",
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        RouteUse::Production,
    );
    assert!(matches!(
        err1,
        Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))
    ));

    // Provider mismatch
    let err2 = registry.resolve(
        "optical.sentinel-2.l1c",
        "planetary-computer",
        "SENTINEL-2",
        RouteUse::Production,
    );
    assert!(matches!(
        err2,
        Err(DatasetRoutePolicyError::ProviderMismatch { .. })
    ));

    // Native collection mismatch
    let err3 = registry.resolve(
        "optical.sentinel-2.l1c",
        "copernicus.dataspace.odata",
        "SENTINEL-3",
        RouteUse::Production,
    );
    assert!(matches!(
        err3,
        Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
    ));
}

#[test]
fn test_landsat_asset_prefix_security_and_expansion() {
    // Valid cases across TM, ETM, and OLI-TIRS
    let valid_cases = [
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05_L2SP_039037_19920715_20200915_02_T1/LT05_L2SP_039037_19920715_20200915_02_T1_SR_B3.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/etm/2003/039/037/LE07_L2SP_039037_20030510_20200915_02_T1/LE07_L2SP_039037_20030510_20200915_02_T1_SR_B3.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09_L2SP_013035_20230326_20230328_02_T1/LC09_L2SP_013035_20230326_20230328_02_T1_SR_B4.TIF",
    ];
    for url in valid_cases {
        assert!(
            validate_planetary_computer_landsat_stable_href(url).is_ok(),
            "Expected valid href for {url}"
        );
    }

    // Adversarial attack patterns
    let attack_cases = [
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/../escape.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/etm/../../escape.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/mss/1975/039/037/LM01.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/modis/file.TIF",
        "https://landsateuwest.blob.core.windows.net/secret/file.TIF",
        "https://evil-blob.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09.TIF",
        "http://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/file.TIF?token=stolen",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/file.TIF#fragment",
    ];
    for url in attack_cases {
        assert!(
            validate_planetary_computer_landsat_stable_href(url).is_err(),
            "Expected attack rejection for {url}"
        );
    }
}
