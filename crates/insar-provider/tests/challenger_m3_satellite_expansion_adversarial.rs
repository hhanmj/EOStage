//! Empirical Challenger Test Suite for Milestone 3 (M3 Satellite Expansion)
//! Tests Landsat 4/5/7/8/9 product identities, prefixes, USGS M2M datasets,
//! invalid IDs, CDSE Sentinel-2/3 profiles, and asset URL validation.

use insar_provider::catalog::{CollectionProfileRegistry, SENTINEL2_L1C_PROFILE_ID};
use insar_provider::cdse::archive_filename::{
    archive_filename_from_odata_name, evaluate_archive_filename, safe_cdse_archive_basename,
    safe_cdse_odata_name,
};
use insar_provider::planetary_computer::{
    validate_planetary_computer_landsat_source_id, validate_planetary_computer_landsat_stable_href,
};
use insar_provider::usgs_m2m::{
    is_supported_landsat_c2_l2_bundle_option, landsat_c2_l2_bundle_option_rank,
    LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM, LANDSAT_C2_L2_BUNDLE_OPTION_ID,
    LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
    LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM, LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID,
    LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE,
};

#[test]
fn test_landsat_identities_validation_and_rejections() {
    // Valid Landsat 4, 5, 7, 8, 9 display IDs and their expected STAC item IDs
    let valid_cases = [
        (
            "LT04_L2SP_039037_19900602_20200915_02_T1",
            "LT04_L2SP_039037_19900602_02_T1",
        ),
        (
            "LT05_L2SP_039037_19920715_20200915_02_T1",
            "LT05_L2SP_039037_19920715_02_T1",
        ),
        (
            "LE07_L2SP_039037_20030510_20200915_02_T1",
            "LE07_L2SP_039037_20030510_02_T1",
        ),
        (
            "LC08_L2SP_039037_20150820_20200915_02_T1",
            "LC08_L2SP_039037_20150820_02_T1",
        ),
        (
            "LC09_L2SP_039037_20220201_20220202_02_T1",
            "LC09_L2SP_039037_20220201_02_T1",
        ),
    ];

    for (display_id, expected_stac_id) in valid_cases {
        assert!(
            validate_planetary_computer_landsat_source_id(expected_stac_id, display_id).is_ok(),
            "Expected valid STAC source id mapping for {display_id}"
        );
    }

    // Invalid IDs that MUST be rejected:
    // LM01 (Landsat 1 MSS), LC04 (invalid sensor/spacecraft combination), malformed, out-of-range dates
    let invalid_cases = [
        "LM01_L2SP_039037_19750602_20200915_02_T1",
        "LM02_L2SP_039037_19750602_20200915_02_T1",
        "LC04_L2SP_039037_19900602_20200915_02_T1",
        "LT08_L2SP_039037_20150820_20200915_02_T1",
        "LE08_L2SP_039037_20150820_20200915_02_T1",
        "LT05_L1TP_039037_19920715_20200915_02_T1",
        "LT05_L2SP_039037_19920715_20200915_01_T1", // collection 01 instead of 02
        "LT05_L2SP_039037_19920715_20200915_02_RT", // RT instead of T1/T2
        "LT05_L2SP_039037_19920230_20200915_02_T1", // invalid date Feb 30
        "malformed_id_without_separators",
        "",
        "   ",
    ];

    for invalid_id in invalid_cases {
        assert!(
            validate_planetary_computer_landsat_source_id("any_item_id", invalid_id).is_err(),
            "Expected rejection for invalid ID {invalid_id}"
        );
    }
}

#[test]
fn test_usgs_m2m_bundle_ranks_and_options() {
    // Current OLI/TIRS bundle: rank 0
    let rank_ot_current = landsat_c2_l2_bundle_option_rank(
        LANDSAT_C2_L2_CURRENT_BUNDLE_OPTION_ID,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE,
        LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM,
    );
    assert_eq!(rank_ot_current, Some(0));

    // Legacy OLI/TIRS bundle: rank 1
    let rank_ot_legacy = landsat_c2_l2_bundle_option_rank(
        LANDSAT_C2_L2_BUNDLE_OPTION_ID,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
        LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
    );
    assert_eq!(rank_ot_legacy, Some(1));

    // TM bundle: code D692, rank 2
    let rank_tm = landsat_c2_l2_bundle_option_rank(
        "custom_tm_option_id",
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        "D692",
        "dds_ms",
    );
    assert_eq!(rank_tm, Some(2));
    assert!(is_supported_landsat_c2_l2_bundle_option(
        "custom_tm_option_id",
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        "D692",
        "dds_ms",
    ));

    // ETM+ bundle: code D693, rank 3
    let rank_etm = landsat_c2_l2_bundle_option_rank(
        "custom_etm_option_id",
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        "D693",
        "dds_ms",
    );
    assert_eq!(rank_etm, Some(3));
    assert!(is_supported_landsat_c2_l2_bundle_option(
        "custom_etm_option_id",
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        "D693",
        "dds_ms",
    ));

    // Unsupported product codes
    assert!(landsat_c2_l2_bundle_option_rank(
        "opt_id",
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
        "D999",
        "dds_ms"
    )
    .is_none());

    // Wrong product name
    assert!(landsat_c2_l2_bundle_option_rank(
        "custom_tm_option_id",
        "Wrong Product Name",
        "D692",
        "dds_ms"
    )
    .is_none());
}

#[test]
fn test_cdse_sentinel2_and_sentinel3_odata_and_archive_filenames() {
    // 1. Sentinel-2 L1C / L2A .SAFE names
    let s2_name = "S2A_MSIL1C_20240101T000000_N0510_R001_T31TGL_20240101T010000.SAFE";
    assert!(safe_cdse_odata_name(s2_name));
    assert!(safe_cdse_archive_basename(&format!("{s2_name}.zip")));

    let s2_ev = archive_filename_from_odata_name(s2_name).expect("S2 archive derivation");
    assert_eq!(s2_ev.basename, format!("{s2_name}.zip"));

    // 2. Sentinel-3 OLCI / SLSTR / SRAL .SEN3 names
    let s3_slstr = "S3A_SL_2_LST____20240101T000000_20240101T000300_20240101T020000_0180_107_001______LN2_O_NR_004.SEN3";
    assert!(
        safe_cdse_odata_name(s3_slstr),
        "S3 SLSTR must be safe OData name"
    );
    assert!(
        safe_cdse_archive_basename(&format!("{s3_slstr}.zip")),
        "S3 SLSTR archive basename must be safe"
    );

    let s3_ev = archive_filename_from_odata_name(s3_slstr).expect("S3 archive derivation");
    assert_eq!(s3_ev.basename, format!("{s3_slstr}.zip"));

    let header_ok = format!("attachment; filename=\"{s3_slstr}.zip\"");
    let eval_res = evaluate_archive_filename(s3_slstr, Some(&header_ok));
    assert!(
        eval_res.is_ok(),
        "evaluate_archive_filename must accept S3 header"
    );

    // Invalid / unsafe formats
    assert!(!safe_cdse_odata_name("product.tar"));
    assert!(!safe_cdse_odata_name("product.zip"));
    assert!(!safe_cdse_odata_name("product.SAFE.SAFE"));
    assert!(!safe_cdse_odata_name("product.SEN3.SEN3"));
    assert!(!safe_cdse_odata_name(""));
    assert!(!safe_cdse_odata_name("../escape.SAFE"));
    assert!(!safe_cdse_archive_basename("product.SEN3.tar"));
}

#[test]
fn test_cdse_collection_profiles_registration() {
    let registry = CollectionProfileRegistry;

    // Sentinel-2 L1C
    let s2_l1c = registry
        .get(SENTINEL2_L1C_PROFILE_ID)
        .expect("Sentinel-2 L1C profile must exist");
    assert!(
        s2_l1c.production_enabled,
        "Sentinel-2 L1C profile must be production_enabled = true"
    );
    assert_eq!(s2_l1c.required_product_type, "S2MSI1C");

    // Sentinel-3 Standard
    let s3_standard = registry
        .get("cdse.sentinel-3.standard.collection.v1")
        .expect("Sentinel-3 standard profile must exist");
    assert!(s3_standard.production_enabled);
    assert_eq!(s3_standard.native_collection_id, "SENTINEL-3");

    // Sentinel-3 SLSTR
    let s3_slstr = registry
        .get("cdse.sentinel-3.slstr.collection.v1")
        .expect("Sentinel-3 SLSTR profile must exist");
    assert_eq!(s3_slstr.required_product_type, "SL_2_LST___");
    assert!(s3_slstr.production_enabled);

    // Sentinel-3 OLCI
    let s3_olci = registry
        .get("cdse.sentinel-3.olci.collection.v1")
        .expect("Sentinel-3 OLCI profile must exist");
    assert_eq!(s3_olci.required_product_type, "OL_2_LFR___");
    assert!(s3_olci.production_enabled);

    // Sentinel-3 SRAL
    let s3_sral = registry
        .get("cdse.sentinel-3.sral.collection.v1")
        .expect("Sentinel-3 SRAL profile must exist");
    assert_eq!(s3_sral.required_product_type, "SR_2_LAN___");
    assert!(s3_sral.production_enabled);
}

#[test]
fn test_empirical_probe_planetary_computer_landsat_tm_etm_prefix_bug() {
    // 1. A Landsat 8/9 URL starts with /landsat-c2/level-2/standard/oli-tirs/ and succeeds
    let l8_href = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2023/013/035/LC09_L2SP_013035_20230326_20230328_02_T1/LC09_L2SP_013035_20230326_20230328_02_T1_SR_B4.TIF";
    assert!(
        validate_planetary_computer_landsat_stable_href(l8_href).is_ok(),
        "Landsat 8/9 asset URL with oli-tirs must be accepted"
    );

    // 2. Landsat 4/5 (tm) URL has path /landsat-c2/level-2/standard/tm/...
    // BUG REPRODUCTION: validate_asset_origin_and_prefix checks
    // url.path().starts_with(PLANETARY_COMPUTER_LANDSAT_ASSET_PREFIX) where
    // PLANETARY_COMPUTER_LANDSAT_ASSET_PREFIX is "/landsat-c2/level-2/standard/oli-tirs/".
    // Therefore, all Landsat 4/5 asset URLs fail validation with IdentityMismatch!
    let l5_href = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/tm/1992/039/037/LT05_L2SP_039037_19920715_20200915_02_T1/LT05_L2SP_039037_19920715_20200915_02_T1_SR_B3.TIF";
    let l5_href_res = validate_planetary_computer_landsat_stable_href(l5_href);
    assert!(
        l5_href_res.is_ok(),
        "L5 href must be accepted now that tm prefix is supported"
    );

    // 3. Landsat 7 (etm) URL has path /landsat-c2/level-2/standard/etm/...
    let l7_href = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/etm/2003/039/037/LE07_L2SP_039037_20030510_20200915_02_T1/LE07_L2SP_039037_20030510_20200915_02_T1_SR_B3.TIF";
    let l7_href_res = validate_planetary_computer_landsat_stable_href(l7_href);
    assert!(
        l7_href_res.is_ok(),
        "L7 href must be accepted now that etm prefix is supported"
    );
}
