use std::fs;
use std::path::PathBuf;

use insar_core::{AsfRawFeature, AssetRole};
use insar_provider::asf::metadata_mapper::map_sar_feature;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    dataset: String,
    feature: AsfRawFeature,
    expected: Option<Expected>,
    error_code: Option<String>,
}

#[derive(Deserialize)]
struct Expected {
    platform: Option<String>,
    processing_level: Option<String>,
    product_family: String,
    path: Option<i64>,
    frame: Option<i64>,
}

#[test]
fn maps_structured_asf_metadata_for_every_initial_dataset() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_asf/sar_metadata_cases.json");
    let fixture: Fixture = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();

    assert_eq!(fixture.cases.len(), 8);
    for case in fixture.cases {
        let feature = if case.dataset == "SEASAT" {
            serde_json::from_str(include_str!(
                "fixtures/asf-seasat/SS_01502_STD_F2536.geojson"
            ))
            .expect("real ASF SEASAT fixture")
        } else {
            case.feature
        };
        let result = map_sar_feature(&case.dataset, &feature);
        if let Some(error_code) = case.error_code {
            assert_eq!(result.unwrap_err().code, error_code, "{}", case.name);
            continue;
        }
        let product = result.unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let expected = case
            .expected
            .expect("successful fixture has expected fields");
        assert_eq!(product.dataset, case.dataset, "{}", case.name);
        assert_eq!(
            product.provider, "asf.catalog",
            "{} must use the canonical provider identity admitted by ApplicationService",
            case.name
        );
        assert_eq!(product.platform, expected.platform, "{}", case.name);
        assert_eq!(
            product.processing_level, expected.processing_level,
            "{}",
            case.name
        );
        assert_eq!(
            product.product_family, expected.product_family,
            "{}",
            case.name
        );
        assert_eq!(product.path, expected.path, "{}", case.name);
        assert_eq!(product.frame, expected.frame, "{}", case.name);
        assert_eq!(
            product.provider_product_key.as_ref(),
            Some(&product.product_key),
            "{}",
            case.name
        );
        assert!(product.native_metadata.contains_key("properties"));
    }
}

#[test]
fn mapper_preserves_geometry_filename_url_and_size_without_name_parsing() {
    let feature: AsfRawFeature = serde_json::from_value(serde_json::json!({
        "type": "Feature",
        "properties": {
            "granuleName": "NOT_A_SENTINEL_NAME",
            "fileID": "NOT_A_SENTINEL_FILE",
            "platform": "ALOS",
            "processingLevel": "L1.5",
            "fileName": "native.img",
            "url": "https://datapool.asf.alaska.edu/native.img",
            "bytes": "2048"
        },
        "geometry": {"type": "Polygon", "coordinates": [[[1,2],[3,2],[3,4],[1,4],[1,2]]]}
    }))
    .unwrap();

    let product = map_sar_feature("ALOS PALSAR", &feature).unwrap();

    assert_eq!(product.product_id, "NOT_A_SENTINEL_FILE");
    assert_eq!(
        product.product_key,
        "PROVIDER::asf.catalog::COLLECTION::ALOS PALSAR::ITEM::NOT_A_SENTINEL_FILE"
    );
    assert_eq!(product.file_name.as_deref(), Some("native.img"));
    assert_eq!(product.file_size_remote, Some(2048));
    assert_eq!(product.footprint_bbox.unwrap().west, 1.0);
    let descriptor = product.assets[0]
        .access_descriptor
        .as_ref()
        .expect("eligible stable ASF URL receives a durable descriptor");
    assert_eq!(descriptor.resolver_id, "earthdata_https");
    assert_eq!(
        descriptor.credential_profile.as_deref(),
        Some("earthdata-default")
    );
    assert_eq!(
        descriptor.stable_href.as_deref(),
        Some("https://datapool.asf.alaska.edu/native.img")
    );
}

fn seasat_fixture() -> AsfRawFeature {
    serde_json::from_str(include_str!(
        "fixtures/asf-seasat/SS_01502_STD_F2536.geojson"
    ))
    .unwrap()
}

fn asset_names(product: &insar_core::SarProduct, role: AssetRole) -> Vec<String> {
    product
        .assets
        .iter()
        .filter(|asset| asset.role == role)
        .filter_map(|asset| asset.file_name.clone())
        .collect()
}

#[test]
fn seasat_mapper_freezes_the_unique_hdf5_primary_and_complete_typed_asset_graph() {
    let product = map_sar_feature("SEASAT", &seasat_fixture()).unwrap();

    assert_eq!(product.processing_level, None);
    assert_eq!(product.product_family, "OTHER");
    assert_eq!(product.file_name.as_deref(), Some("SS_01502_STD_F2536.h5"));
    assert_eq!(product.file_size_remote, Some(425_004_910));
    assert_eq!(
        product.checksum.as_deref(),
        Some("fd99f89b380163d20c50b4a26fa0db12")
    );
    assert_eq!(
        asset_names(&product, AssetRole::Primary),
        ["SS_01502_STD_F2536.h5"]
    );
    assert_eq!(
        asset_names(&product, AssetRole::Derived),
        ["SS_01502_STD_F2536.tif"]
    );
    assert_eq!(
        asset_names(&product, AssetRole::Browse),
        ["SS_01502_STD_F2536.jpg"]
    );
    assert_eq!(
        asset_names(&product, AssetRole::Thumbnail),
        ["SS_01502_STD_F2536_THUMBNAIL.jpg"]
    );
    assert_eq!(
        asset_names(&product, AssetRole::Metadata),
        [
            "SS_01502_STD_F2536.iso.xml",
            "SS_01502_STD_F2536.kml",
            "SS_01502_STD_F2536.xml",
        ]
    );
    assert_eq!(
        asset_names(&product, AssetRole::Auxiliary),
        ["SS_01502_STD_F2536.qc_report", "SS_01502_STD_F2536.roi.in",]
    );
    assert_eq!(product.assets.len(), 9);
    let primary = product
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .unwrap();
    assert_eq!(primary.size_bytes, Some(425_004_910));
}

#[test]
fn seasat_asset_order_is_stable_when_provider_url_order_changes() {
    let original = seasat_fixture();
    let expected = map_sar_feature("SEASAT", &original).unwrap();
    let mut value = serde_json::to_value(original).unwrap();
    value["properties"]["additionalUrls"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let shuffled: AsfRawFeature = serde_json::from_value(value).unwrap();
    let actual = map_sar_feature("SEASAT", &shuffled).unwrap();
    assert_eq!(
        actual
            .assets
            .iter()
            .map(|asset| (&asset.role, &asset.file_name))
            .collect::<Vec<_>>(),
        expected
            .assets
            .iter()
            .map(|asset| (&asset.role, &asset.file_name))
            .collect::<Vec<_>>(),
    );
    assert_eq!(actual.file_name, expected.file_name);
    assert_eq!(actual.download_url, expected.download_url);
}

#[test]
fn seasat_missing_or_ambiguous_hdf5_fails_closed_without_geotiff_fallback() {
    let value = serde_json::to_value(seasat_fixture()).unwrap();

    let mut missing = value.clone();
    missing["properties"]["fileName"] = Value::String("SS_01502_STD_F2536.tif".into());
    missing["properties"]["url"] = Value::String(
        "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536.tif".into(),
    );
    let error = map_sar_feature("SEASAT", &serde_json::from_value(missing).unwrap()).unwrap_err();
    assert_eq!(error.code, "ASF_PRIMARY_ASSET_MISSING");

    let mut ambiguous = value;
    ambiguous["properties"]["additionalUrls"]
        .as_array_mut()
        .unwrap()
        .push(Value::String(
            "https://cumulus.asf.earthdatacloud.nasa.gov/SEASAT/SS_01502_STD_F2536/SS_01502_STD_F2536_ALT.hdf5".into(),
        ));
    ambiguous["properties"]["bytes"]["SS_01502_STD_F2536_ALT.hdf5"] =
        serde_json::json!({"bytes": 1, "format": "HDF5"});
    let error = map_sar_feature("SEASAT", &serde_json::from_value(ambiguous).unwrap()).unwrap_err();
    assert_eq!(error.code, "ASF_PRIMARY_ASSET_AMBIGUOUS");
}

#[test]
fn seasat_missing_browse_keeps_hdf5_primary_without_promoting_jpegs() {
    let mut value = serde_json::to_value(seasat_fixture()).unwrap();
    value["properties"]
        .as_object_mut()
        .unwrap()
        .remove("browse");
    let product = map_sar_feature("SEASAT", &serde_json::from_value(value).unwrap()).unwrap();
    assert_eq!(
        asset_names(&product, AssetRole::Primary),
        ["SS_01502_STD_F2536.h5"]
    );
    assert!(asset_names(&product, AssetRole::Browse).is_empty());
    assert!(asset_names(&product, AssetRole::Thumbnail).is_empty());
}

#[test]
fn mapper_preserves_provider_browse_as_a_separate_preview_asset() {
    let feature: AsfRawFeature = serde_json::from_value(serde_json::json!({
        "type": "Feature",
        "properties": {
            "granuleName": "S1A_PREVIEW_FIXTURE",
            "fileID": "S1A_PREVIEW_FIXTURE",
            "platform": "Sentinel-1A",
            "processingLevel": "GRD_HD",
            "fileName": "S1A_PREVIEW_FIXTURE.zip",
            "url": "https://datapool.asf.alaska.edu/GRD_HD/S1A_PREVIEW_FIXTURE.zip",
            "browse": ["https://datapool.asf.alaska.edu/BROWSE/S1A_PREVIEW_FIXTURE.jpg"],
            "bytes": "2048"
        },
        "geometry": null
    }))
    .unwrap();

    let product = map_sar_feature("SENTINEL-1", &feature).unwrap();

    assert_eq!(product.assets.len(), 2);
    assert_eq!(product.assets[0].role, insar_core::AssetRole::Primary);
    let browse = &product.assets[1];
    assert_eq!(browse.role, insar_core::AssetRole::Browse);
    assert_eq!(
        browse.url.as_deref(),
        Some("https://datapool.asf.alaska.edu/BROWSE/S1A_PREVIEW_FIXTURE.jpg")
    );
    assert_eq!(browse.media_type.as_deref(), Some("image/jpeg"));
    assert_eq!(
        browse.asset_key.as_deref(),
        Some("S1A_PREVIEW_FIXTURE:browse")
    );
    assert!(browse
        .stable_asset_key
        .as_deref()
        .is_some_and(|value| value.ends_with("::ASSET::S1A_PREVIEW_FIXTURE:browse")));
}

/// Recorded 2026-10-20T03:37Z from the anonymous ASF search API (dataset
/// SENTINEL-1, processingLevel SLC, output geojson, maxResults 3, the AOI
/// W105.4930 E105.6289 S24.6707 N24.7880, 2026-08-01..2026-10-19). The
/// Sentinel-1D scenes carry no lookDirection; the SLC mapper keeps it
/// optional, so the burst defect of the same day does not reach SLC search.
#[test]
fn recorded_sentinel_1d_slc_scenes_map_with_their_identity() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/asf/sentinel-1-slc-recorded-2026-10-20.geojson"
    ))
    .unwrap();
    let features = fixture["features"].as_array().unwrap();
    assert_eq!(features.len(), 3);
    for feature in features {
        let raw: AsfRawFeature = serde_json::from_value(feature.clone()).unwrap();
        let product = map_sar_feature("SENTINEL-1", &raw).unwrap();
        assert_eq!(product.platform.as_deref(), Some("Sentinel-1D"));
        assert_eq!(product.processing_level.as_deref(), Some("SLC"));
        assert!(product.path.is_some_and(|path| path > 0));
        assert!(product.absolute_orbit.is_some_and(|orbit| orbit > 0));
        assert_eq!(product.look_direction, None, "never inferred");
        assert!(product.footprint_geojson.is_some());
    }
}
