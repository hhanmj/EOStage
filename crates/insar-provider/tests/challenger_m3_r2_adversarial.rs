//! Adversarial Empirical Challenger Suite — Milestone 3 Remediation (M3-R2)
//! Stress-testing edge cases, injection attempts, prefix boundaries, and contract invariants.

use insar_core::CatalogSearchQuery;
use insar_provider::catalog::RouteUse;
use insar_provider::catalog::{
    CollectionProfileRegistry, SENTINEL2_L1C_PROFILE_ID, SENTINEL3_OLCI_PROFILE,
    SENTINEL3_OLCI_PROFILE_ID, SENTINEL3_SLSTR_PROFILE, SENTINEL3_SLSTR_PROFILE_ID,
    SENTINEL3_SRAL_PROFILE, SENTINEL3_SRAL_PROFILE_ID, SENTINEL3_STANDARD_PROFILE,
    SENTINEL3_STANDARD_PROFILE_ID,
};
use insar_provider::cdse::query::build_odata_request_for_use;
use insar_provider::planetary_computer::{
    is_valid_planetary_computer_landsat_path, validate_planetary_computer_landsat_stable_href,
};
use insar_provider::usgs_m2m::{
    landsat_c2_l2_bundle_option_rank, live_identity_for_alias, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
    LANDSAT_ETM_C2_L2_BUNDLE_PRODUCT_CODE, LANDSAT_TM_C2_L2_BUNDLE_PRODUCT_CODE,
    USGS_M2M_LANDSAT_ETM_C2_L2_LIVE_IDENTITY, USGS_M2M_LANDSAT_TM_C2_L2_LIVE_IDENTITY,
};
use serde_json::json;

#[test]
fn adversarial_planetary_computer_landsat_path_boundary_and_traversal() {
    // 1. Valid prefixes with valid full paths
    let valid_full_paths = [
        "/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09_L2SP_013035_20230326_20230328_02_T1/LC09_L2SP_013035_20230326_20230328_02_T1_SR_B4.TIF",
        "/landsat-c2/level-2/standard/tm/1992/039/037/LT05_L2SP_039037_19920715_20200915_02_T1/LT05_L2SP_039037_19920715_20200915_02_T1_SR_B3.TIF",
        "/landsat-c2/level-2/standard/etm/2003/039/037/LE07_L2SP_039037_20030510_20200915_02_T1/LE07_L2SP_039037_20030510_20200915_02_T1_SR_B3.TIF",
    ];
    for path in valid_full_paths {
        assert!(
            is_valid_planetary_computer_landsat_path(path),
            "Expected valid path for {path}"
        );
        let href = format!("https://landsateuwest.blob.core.windows.net{path}");
        assert!(
            validate_planetary_computer_landsat_stable_href(&href).is_ok(),
            "Expected valid href for {href}"
        );
    }

    // 2. Adversarial prefix collision attempts: must be rejected
    let adversarial_collision_paths = [
        "/landsat-c2/level-2/standard/tm_attacker/hack.TIF",
        "/landsat-c2/level-2/standard/etm_attacker/hack.TIF",
        "/landsat-c2/level-2/standard/oli-tirs_attacker/hack.TIF",
        "/landsat-c2/level-2/standard/mss/1975/001/001/LM01.TIF",
        "/landsat-c2/level-2/standard/tm", // missing trailing slash
        "/landsat-c2/level-2/standard/etm",
        "/landsat-c2/level-2/standard/oli-tirs",
        "/landsat-c2/level-2/standard/",
        "",
        "/",
    ];
    for path in adversarial_collision_paths {
        assert!(
            !is_valid_planetary_computer_landsat_path(path),
            "Adversarial collision path must NOT be accepted: {path}"
        );
        let href = format!("https://landsateuwest.blob.core.windows.net{path}");
        assert!(
            validate_planetary_computer_landsat_stable_href(&href).is_err(),
            "Expected rejection for href: {href}"
        );
    }

    // 3. Path traversal, URL injection, and scheme/host tampering
    let tampering_hrefs = [
        // Path traversal out of valid sensor folders
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/../escape.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/etm/../../escape.TIF",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/../../../escape.TIF",
        // Query param and fragment tampering
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF?token=secret",
        "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF#hash",
        // Host & scheme tampering
        "http://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF",
        "https://evil.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF",
        "https://landsateuwest.blob.core.windows.net:8443/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF",
        "https://user:pass@landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05.TIF",
    ];
    for href in tampering_hrefs {
        assert!(
            validate_planetary_computer_landsat_stable_href(href).is_err(),
            "Tampering attempt must be rejected: {href}"
        );
    }
}

fn make_s3_query(
    product_type: serde_json::Value,
    spacecraft: Option<serde_json::Value>,
) -> CatalogSearchQuery {
    let mut filters = serde_json::Map::new();
    filters.insert("product_type".to_string(), product_type);
    filters.insert(
        "datetime".to_string(),
        json!({"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}),
    );
    if let Some(sc) = spacecraft {
        filters.insert("spacecraft".to_string(), sc);
    }

    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-3.standard",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-3",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "cursor": null,
        "result_limit": 20,
        "filters": filters
    }))
    .expect("valid base query structure")
}

#[test]
fn adversarial_cdse_sentinel3_query_builder_injection_and_boundaries() {
    // 1. Single product type in array
    let q1 = make_s3_query(json!(["SL_2_LST___"]), None);
    let req1 = build_odata_request_for_use(&q1, RouteUse::Production).unwrap();
    let filter1 = get_odata_filter(&req1);
    assert!(filter1.contains("Collection/Name eq 'SENTINEL-3'"));
    assert!(filter1.contains("Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'SL_2_LST___')"));

    // 2. Multi product types in array: live CDSE OData v4 rejects nested `(att/... or ...)`
    // inside a single `any(att:...)` with HTTP 400 `Invalid field: att`, so each productType
    // must be emitted as its own `any(...)` clause joined by top-level `or`.
    let q2 = make_s3_query(json!(["OL_1_EFR___", "SL_1_RBT___", "SR_2_LAN___"]), None);
    let req2 = build_odata_request_for_use(&q2, RouteUse::Production).unwrap();
    let filter2 = get_odata_filter(&req2);
    assert!(filter2.contains("(Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'OL_1_EFR___') or Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'SL_1_RBT___') or Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'SR_2_LAN___'))"));

    // 3. Array deduplication: identical types must collapse into a single condition
    let q3 = make_s3_query(json!(["SL_2_LST___", "SL_2_LST___"]), None);
    let req3 = build_odata_request_for_use(&q3, RouteUse::Production).unwrap();
    let filter3 = get_odata_filter(&req3);
    assert!(filter3.contains("Attributes/OData.CSC.StringAttribute/any(att:att/Name eq 'productType' and att/OData.CSC.StringAttribute/Value eq 'SL_2_LST___')"));
    assert!(!filter3.contains(" or "));

    // 4. Adversarial injections in product_type: must fail-closed
    let malicious_types = [
        json!(["SL_2_LST___' or '1'='1"]),
        json!(["SL_2_LST___'; DROP TABLE products;--"]),
        json!([""]),
        json!(["   "]),
        json!(["sl_2_lst___"]), // lower-case must not be accepted
        json!(["OL_9_XYZ___"]), // unknown type
        json!([]),              // empty array
        json!(["OL_1_EFR___", "MALICIOUS_INJECTION"]), // partial malicious
        json!(12345),           // integer instead of string/array
        json!(true),            // boolean
        json!({"type": "SL_2_LST___"}), // object
    ];
    for bad_type in malicious_types {
        let q_bad = make_s3_query(bad_type.clone(), None);
        assert!(
            build_odata_request_for_use(&q_bad, RouteUse::Production).is_err(),
            "Expected failure for malicious or invalid product_type: {bad_type:?}"
        );
    }

    // 5. Sentinel-3 spacecraft filter adversarial testing
    // Valid:
    let q_sc_valid = make_s3_query(
        json!(["SL_2_LST___"]),
        Some(json!(["sentinel_3a", "sentinel_3b"])),
    );
    assert!(build_odata_request_for_use(&q_sc_valid, RouteUse::Production).is_ok());

    // Invalid spacecraft: duplicates, unknown platform, empty list
    let bad_spacecraft = [
        json!(["sentinel_3a", "sentinel_3a"]), // duplicate
        json!(["sentinel_3c"]),                // unknown platform
        json!(["sentinel_2a"]),                // cross-satellite platform
        json!([]),                             // empty array
    ];
    for bad_sc in bad_spacecraft {
        let q_sc_bad = make_s3_query(json!(["SL_2_LST___"]), Some(bad_sc.clone()));
        assert!(
            build_odata_request_for_use(&q_sc_bad, RouteUse::Production).is_err(),
            "Expected rejection for bad spacecraft: {bad_sc:?}"
        );
    }
}

fn get_odata_filter(req: &insar_provider::cdse::query::ODataRequest) -> String {
    url::form_urlencoded::parse(
        req.path_and_query()
            .split_once('?')
            .expect("query string")
            .1
            .as_bytes(),
    )
    .find(|(k, _)| k == "$filter")
    .expect("$filter param")
    .1
    .into_owned()
}

#[test]
fn adversarial_usgs_m2m_reexports_and_bundle_ranks() {
    // 1. Re-exports check
    assert_eq!(LANDSAT_TM_C2_L2_BUNDLE_PRODUCT_CODE, "D692");
    assert_eq!(LANDSAT_ETM_C2_L2_BUNDLE_PRODUCT_CODE, "D693");
    assert_eq!(
        USGS_M2M_LANDSAT_TM_C2_L2_LIVE_IDENTITY
            .unwrap()
            .dataset_alias(),
        "landsat_tm_c2_l2"
    );
    assert_eq!(
        USGS_M2M_LANDSAT_ETM_C2_L2_LIVE_IDENTITY
            .unwrap()
            .dataset_alias(),
        "landsat_etm_c2_l2"
    );

    // 2. Alias resolution
    let tm_id = live_identity_for_alias("landsat_tm_c2_l2").expect("tm identity");
    assert_eq!(tm_id.dataset_alias(), "landsat_tm_c2_l2");
    assert_eq!(tm_id.collection_name(), "Landsat 4-5 TM C2 L2");

    let etm_id = live_identity_for_alias("landsat_etm_c2_l2").expect("etm identity");
    assert_eq!(etm_id.dataset_alias(), "landsat_etm_c2_l2");
    assert_eq!(etm_id.collection_name(), "Landsat 7 ETM+ C2 L2");

    // Unknown alias fails closed
    assert!(live_identity_for_alias("unknown_alias").is_none());
    assert!(live_identity_for_alias("landsat_mss_c2_l2").is_none());

    // 3. Bundle rank hierarchy
    // Rank 2: TM
    assert_eq!(
        landsat_c2_l2_bundle_option_rank(
            "opt",
            LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "D692",
            "dds_ms"
        ),
        Some(2)
    );
    // Rank 3: ETM+
    assert_eq!(
        landsat_c2_l2_bundle_option_rank(
            "opt",
            LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "D693",
            "dds_ms"
        ),
        Some(3)
    );
    // Unsupported code
    assert_eq!(
        landsat_c2_l2_bundle_option_rank(
            "opt",
            LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "D999",
            "dds_ms"
        ),
        None
    );
    // Unsupported system
    assert_eq!(
        landsat_c2_l2_bundle_option_rank(
            "opt",
            LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "D692",
            "invalid_sys"
        ),
        None
    );
}

#[test]
fn adversarial_catalog_reexports_and_profiles() {
    assert_eq!(
        SENTINEL3_OLCI_PROFILE_ID,
        "cdse.sentinel-3.olci.collection.v1"
    );
    assert_eq!(
        SENTINEL3_SLSTR_PROFILE_ID,
        "cdse.sentinel-3.slstr.collection.v1"
    );
    assert_eq!(
        SENTINEL3_SRAL_PROFILE_ID,
        "cdse.sentinel-3.sral.collection.v1"
    );
    assert_eq!(
        SENTINEL3_STANDARD_PROFILE_ID,
        "cdse.sentinel-3.standard.collection.v1"
    );

    assert_eq!(SENTINEL3_OLCI_PROFILE.required_product_type, "OL_2_LFR___");
    assert_eq!(SENTINEL3_SLSTR_PROFILE.required_product_type, "SL_2_LST___");
    assert_eq!(SENTINEL3_SRAL_PROFILE.required_product_type, "SR_2_LAN___");
    assert_eq!(
        SENTINEL3_STANDARD_PROFILE.native_collection_id,
        "SENTINEL-3"
    );

    let reg = CollectionProfileRegistry;
    assert!(reg.get(SENTINEL2_L1C_PROFILE_ID).is_some());
    assert!(reg.get(SENTINEL3_OLCI_PROFILE_ID).is_some());
    assert!(reg.get(SENTINEL3_SLSTR_PROFILE_ID).is_some());
    assert!(reg.get(SENTINEL3_SRAL_PROFILE_ID).is_some());
    assert!(reg.get(SENTINEL3_STANDARD_PROFILE_ID).is_some());
}
