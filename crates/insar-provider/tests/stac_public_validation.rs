//! Default-ignored public validation. Metadata probes do not fetch image data;
//! the explicit cross-domain asset test signs reviewed COGs and reads one byte only.
use chrono::DateTime;
use insar_core::{CatalogSearchQuery, DataSourceKind, ProductKind, ProviderNetworkPolicy};
use insar_provider::catalog::{
    MpcReviewedCatalogClient, StacPublicValidationClient, MPC_COPDEM_GLO30_COLLECTION_ID,
    MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_SENTINEL1_GRD_COLLECTION_ID,
    MPC_SENTINEL1_GRD_LOGICAL_DATASET, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_provider::planetary_computer_asset::{
    PlanetaryComputerAssetSigner, PlanetaryComputerReviewedAssetClass,
};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug)]
enum ScienceValueKind {
    String,
    StringOrInteger,
    Number,
}

fn required_science_value<'a>(
    properties: &'a Value,
    key: &str,
    kind: ScienceValueKind,
) -> Result<&'a Value, String> {
    let value = properties
        .get(key)
        .ok_or_else(|| format!("missing required science field {key}"))?;
    if value.is_null() {
        return Err(format!("null required science field {key}"));
    }
    let valid_type = match kind {
        ScienceValueKind::String => value.as_str().is_some(),
        ScienceValueKind::StringOrInteger => {
            value.as_str().is_some() || value.as_i64().is_some() || value.as_u64().is_some()
        }
        ScienceValueKind::Number => value.as_f64().is_some(),
    };
    valid_type
        .then_some(value)
        .ok_or_else(|| format!("wrong type for required science field {key}"))
}

#[test]
fn source_comparison_rejects_missing_null_and_wrong_typed_science_fields() {
    let mut properties = json!({"datetime":"2025-01-11T15:26:27Z"});
    assert!(required_science_value(&properties, "datetime", ScienceValueKind::String).is_ok());
    properties.as_object_mut().unwrap().remove("datetime");
    assert!(required_science_value(&properties, "datetime", ScienceValueKind::String).is_err());
    properties["datetime"] = Value::Null;
    assert!(required_science_value(&properties, "datetime", ScienceValueKind::String).is_err());
    properties["datetime"] = json!(20250111);
    assert!(required_science_value(&properties, "datetime", ScienceValueKind::String).is_err());
}

#[test]
#[ignore = "requires explicit owner approval and EOSTAGE_STAC_PUBLIC_VALIDATION=1; contacts fixed public catalogue"]
fn planetary_computer_sentinel2_two_pages_without_credentials_or_image_cache() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let client = StacPublicValidationClient::planetary_computer_sentinel2().unwrap();
    let response = client.search(&|| false).unwrap();
    response.validate().unwrap();
    assert!(!response.collection.production_enabled);
    assert_eq!(response.collection.native_collection_id, "sentinel-2-l2a");
    assert_eq!(response.products.len(), 4);
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.fetched_record_count, 4);
    assert!(response.next_cursor.is_none());
    assert!(response.stats.working_set_satisfied);
    assert!(!response.stats.truncated);
    let mut items = Vec::new();
    for product in &response.products {
        let raw_time = product.properties["stac_properties"]["datetime"]
            .as_str()
            .unwrap();
        let normalized_time = product.acquisition_datetime.as_deref().unwrap();
        assert_eq!(
            DateTime::parse_from_rfc3339(raw_time).unwrap(),
            DateTime::parse_from_rfc3339(normalized_time).unwrap()
        );
        assert_eq!(
            product.properties["stac_item_id"].as_str(),
            Some(product.product_id.as_str())
        );
        assert!(!product.assets.is_empty());
        assert!(product
            .assets
            .iter()
            .all(|asset| asset.access_descriptor.is_none()));
        items.push(json!({
            "id":product.product_id, "raw_datetime":raw_time,
            "normalized_datetime":normalized_time,
            "geometry_type":product.footprint_geojson.as_ref().unwrap()["type"],
            "descriptive_assets":product.assets.len(),
            "size_is_unknown":product.assets.iter().all(|asset| asset.size_bytes.is_none())
        }));
    }
    println!(
        "STAC_PUBLIC_SUMMARY {}",
        json!({
            "source":"Microsoft Planetary Computer", "collection":"sentinel-2-l2a",
            "requests":response.stats.request_count, "items":items,
            "remote_total":response.stats.matched_count,
            "remote_exhausted":response.stats.remote_exhausted,
            "image_requests":0, "response_cache_files":0,
            "response_byte_limit":1024*1024,
            "production_route_registered":false
        })
    );
}

#[test]
#[ignore = "requires explicit owner approval and EOSTAGE_STAC_PUBLIC_VALIDATION=1; contacts two fixed public catalogues"]
fn landsat_same_scene_matches_microsoft_and_earth_search_without_asset_fetch() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_PUBLIC_VALIDATION").as_deref(),
        Ok("1")
    );
    let microsoft = StacPublicValidationClient::planetary_computer_landsat()
        .unwrap()
        .search_landsat(&|| false)
        .unwrap();
    let earth = StacPublicValidationClient::earth_search_landsat()
        .unwrap()
        .search_landsat(&|| false)
        .unwrap();
    microsoft.validate().unwrap();
    earth.validate().unwrap();
    assert_eq!(microsoft.products.len(), 1);
    assert_eq!(earth.products.len(), 1);
    let left = &microsoft.products[0];
    let right = &earth.products[0];
    assert_eq!(left.product_id, "LC09_L2SP_125038_20250111_02_T1");
    assert_eq!(right.product_id, left.product_id);
    assert_eq!(left.acquisition_datetime, right.acquisition_datetime);
    assert_eq!(left.footprint_geojson, right.footprint_geojson);
    for (key, kind) in [
        ("datetime", ScienceValueKind::String),
        ("landsat:scene_id", ScienceValueKind::String),
        ("landsat:wrs_path", ScienceValueKind::StringOrInteger),
        ("landsat:wrs_row", ScienceValueKind::StringOrInteger),
        ("landsat:correction", ScienceValueKind::String),
        (
            "landsat:collection_number",
            ScienceValueKind::StringOrInteger,
        ),
        ("landsat:collection_category", ScienceValueKind::String),
        ("platform", ScienceValueKind::String),
        ("eo:cloud_cover", ScienceValueKind::Number),
    ] {
        let left_value = required_science_value(&left.properties["stac_properties"], key, kind)
            .unwrap_or_else(|error| panic!("Microsoft: {error}"));
        let right_value = required_science_value(&right.properties["stac_properties"], key, kind)
            .unwrap_or_else(|error| panic!("Earth Search: {error}"));
        assert_eq!(left_value, right_value, "{key}");
    }
    assert!(left.assets.is_empty() && right.assets.is_empty());
    let left_keys = left.payload["stac"]["unmapped_asset_keys"]
        .as_array()
        .unwrap();
    let right_keys = right.payload["stac"]["unmapped_asset_keys"]
        .as_array()
        .unwrap();
    assert!(left_keys.len() >= 20 && right_keys.len() >= 20);
    assert_ne!(
        left_keys, right_keys,
        "preview/tile delivery semantics are source-specific"
    );
    println!(
        "STAC_LANDSAT_SOURCE_COMPARISON {}",
        json!({
            "item_id":left.product_id,
            "scene_id":left.properties["stac_properties"]["landsat:scene_id"],
            "datetime":left.acquisition_datetime,
            "microsoft_requests":microsoft.stats.request_count,
            "earth_search_requests":earth.stats.request_count,
            "microsoft_remote_total":microsoft.stats.matched_count,
            "earth_search_remote_total":earth.stats.matched_count,
            "microsoft_remote_exhausted":microsoft.stats.remote_exhausted,
            "earth_search_remote_exhausted":earth.stats.remote_exhausted,
            "microsoft_asset_keys":left_keys,
            "earth_search_asset_keys":right_keys,
            "mapped_executable_assets":0,
            "image_requests":0,
            "response_cache_files":0,
            "production_route_registered":false
        })
    );
}

#[test]
#[ignore = "requires EOSTAGE_STAC_CROSS_DOMAIN_VALIDATION=1; contacts fixed public MPC catalogue only"]
fn planetary_computer_cross_domain_sar_and_dem_catalogue_assets_are_reviewed() {
    assert_eq!(
        std::env::var("EOSTAGE_STAC_CROSS_DOMAIN_VALIDATION").as_deref(),
        Ok("1")
    );
    let sar = StacPublicValidationClient::planetary_computer_sentinel1_grd()
        .unwrap()
        .search_sentinel1_grd(&|| false)
        .unwrap();
    sar.validate().unwrap();
    assert_eq!(sar.products.len(), 1);
    let sar_product = &sar.products[0];
    assert_eq!(sar_product.source_kind, DataSourceKind::Sentinel1);
    assert_eq!(sar_product.product_kind, ProductKind::SarGrd);
    assert_eq!(sar.collection.native_collection_id, "sentinel-1-grd");
    assert!(sar_product
        .assets
        .iter()
        .all(|asset| asset.access_descriptor.is_none()));
    assert!(sar_product
        .assets
        .iter()
        .any(|asset| asset.asset_key.as_deref() == Some("thumbnail")));
    assert!(sar_product.assets.iter().any(|asset| {
        matches!(
            asset.asset_key.as_deref(),
            Some("vv") | Some("vh") | Some("hh") | Some("hv")
        )
    }));
    assert_eq!(
        sar_product.properties["stac_properties"]["sar:product_type"].as_str(),
        Some("GRD")
    );

    let dem = StacPublicValidationClient::planetary_computer_cop_dem_glo30()
        .unwrap()
        .search_cop_dem_glo30(&|| false)
        .unwrap();
    dem.validate().unwrap();
    assert_eq!(dem.products.len(), 1);
    let dem_product = &dem.products[0];
    assert_eq!(dem_product.source_kind, DataSourceKind::Dem);
    assert_eq!(dem_product.product_kind, ProductKind::DemRaster);
    assert_eq!(dem.collection.native_collection_id, "cop-dem-glo-30");
    assert_eq!(dem_product.assets.len(), 1);
    assert_eq!(dem_product.assets[0].asset_key.as_deref(), Some("data"));
    assert!(dem_product.assets[0].access_descriptor.is_none());
}

#[test]
#[ignore = "requires EOSTAGE_MPC_REVIEWED_SEARCH_LIVE=1; runs bounded production reviewed S1 GRD and COP30 STAC searches"]
fn planetary_computer_reviewed_production_client_returns_only_delivery_candidate_assets() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_REVIEWED_SEARCH_LIVE").as_deref(),
        Ok("1")
    );
    let client = MpcReviewedCatalogClient::new(ProviderNetworkPolicy::direct()).unwrap();
    for (logical_dataset, collection, source_kind, product_kind, expected_keys) in [
        (
            MPC_SENTINEL1_GRD_LOGICAL_DATASET,
            MPC_SENTINEL1_GRD_COLLECTION_ID,
            DataSourceKind::Sentinel1,
            ProductKind::SarGrd,
            &["vv", "vh", "hh", "hv"][..],
        ),
        (
            MPC_COPDEM_GLO30_LOGICAL_DATASET,
            MPC_COPDEM_GLO30_COLLECTION_ID,
            DataSourceKind::Dem,
            ProductKind::DemRaster,
            &["data"][..],
        ),
    ] {
        let aoi_geojson = Some(
            json!({
                "type":"Polygon",
                "coordinates":[[[
                    121.3,24.9
                ],[
                    121.7,24.9
                ],[
                    121.7,25.3
                ],[
                    121.3,25.3
                ],[
                    121.3,24.9
                ]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let filters = if logical_dataset == MPC_SENTINEL1_GRD_LOGICAL_DATASET {
            serde_json::Map::from_iter([(
                "datetime".to_owned(),
                json!({
                    "start":"2025-01-01T00:00:00Z",
                    "end":"2025-01-31T23:59:59Z"
                }),
            )])
        } else {
            serde_json::Map::new()
        };
        let query = CatalogSearchQuery {
            logical_dataset: logical_dataset.to_owned(),
            provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
            native_collection_id: collection.to_owned(),
            aoi_geojson,
            cursor: None,
            result_limit: Some(1),
            filters,
        };
        let response = client.search_catalog(&query, &|| false).unwrap();
        response.validate().unwrap();
        assert_eq!(response.products.len(), 1);
        assert_eq!(response.collection.logical_dataset, logical_dataset);
        assert_eq!(
            response.collection.provider_id,
            PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        );
        assert_eq!(response.collection.native_collection_id, collection);
        let product = &response.products[0];
        assert_eq!(product.source_kind, source_kind);
        assert_eq!(product.product_kind, product_kind);
        assert_eq!(product.product_id, product.display_name);
        assert!(product.properties.is_empty());
        assert!(product.payload.is_empty());
        assert!(!product.assets.is_empty());
        assert!(product.assets.iter().all(|asset| {
            asset
                .asset_key
                .as_deref()
                .is_some_and(|key| expected_keys.contains(&key))
                && asset.access_descriptor.is_none()
                && asset.url.as_deref().is_some_and(|url| !url.contains('?'))
        }));
    }
}

#[test]
#[ignore = "requires EOSTAGE_MPC_CROSS_DOMAIN_ASSET_LIVE=1; signs two reviewed public COG assets and reads one byte from each"]
fn planetary_computer_cross_domain_signer_supports_bounded_cog_range_reads() {
    assert_eq!(
        std::env::var("EOSTAGE_MPC_CROSS_DOMAIN_ASSET_LIVE").as_deref(),
        Ok("1")
    );
    let sar = StacPublicValidationClient::planetary_computer_sentinel1_grd()
        .unwrap()
        .search_sentinel1_grd(&|| false)
        .unwrap();
    let sar_asset = sar.products[0]
        .assets
        .iter()
        .find(|asset| {
            matches!(
                asset.asset_key.as_deref(),
                Some("vv") | Some("vh") | Some("hh") | Some("hv")
            )
        })
        .and_then(|asset| asset.url.as_deref())
        .expect("Sentinel-1 GRD probe returns at least one polarization COG");

    let dem = StacPublicValidationClient::planetary_computer_cop_dem_glo30()
        .unwrap()
        .search_cop_dem_glo30(&|| false)
        .unwrap();
    let dem_asset = dem.products[0].assets[0]
        .url
        .as_deref()
        .expect("COP30 probe returns its data COG");

    let signer = PlanetaryComputerAssetSigner::new(ProviderNetworkPolicy::system()).unwrap();
    for (class, stable_href) in [
        (
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
            sar_asset,
        ),
        (
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
            dem_asset,
        ),
    ] {
        let runtime = signer
            .sign_reviewed_asset(class, stable_href, &|| false)
            .unwrap();
        let (signed, expires_at) = runtime.into_parts();
        assert!(expires_at > std::time::SystemTime::now());
        let response = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(Policy::none())
            .build()
            .unwrap()
            .get(signed)
            .header(RANGE, "bytes=0-0")
            .send()
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.content_length(), Some(1));
        assert!(response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("bytes 0-0/")));
        assert_eq!(response.bytes().unwrap().len(), 1);
    }
}
