use std::fs;
use std::path::PathBuf;

use insar_core::AsfRawFeature;
use insar_provider::asf::collection_profile::{
    profile_for_dataset, profile_for_id, AsfDeliveryModel, AsfItemModel,
};
use insar_provider::asf::identity::map_and_aggregate_sar_features;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    dataset: String,
    features: Vec<AsfRawFeature>,
    expected: Expected,
}

#[derive(Deserialize)]
struct Expected {
    provider_product_ids: Vec<String>,
    product_count: usize,
    asset_count: u64,
    replayed_record_count: u64,
    mapping_error_count: u64,
    identity_conflict_count: u64,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_asf/asf_identity_cases.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn collection_profiles_assign_file_id_by_collection_semantics() {
    let alos = profile_for_dataset("ALOS PALSAR").expect("ALOS profile");
    assert_eq!(alos.item_model, AsfItemModel::OneFilePerProduct);
    assert_eq!(alos.item_id_fields, &["productID", "granuleUR", "fileID"]);
    assert!(alos.asset_id_fields.contains(&"fileID"));

    let ers = profile_for_dataset("ERS").expect("ERS profile");
    assert_eq!(ers.item_model, AsfItemModel::OneFilePerProduct);
    assert!(ers.item_id_fields.contains(&"fileID"));
    assert!(profile_for_dataset("FUTURE-SAR").is_none());
}

#[test]
fn sentinel_standard_profile_declares_reviewed_single_archive_delivery() {
    let profile = profile_for_dataset("SENTINEL-1").expect("Sentinel-1 profile");

    assert_eq!(profile.profile_id, "asf.sentinel-1.standard.v1");
    assert_eq!(profile.rule_id, "asf.sentinel-1.standard.v1");
    assert_eq!(
        profile.delivery_model,
        AsfDeliveryModel::SinglePrimaryArchive
    );
    assert_eq!(profile.approved_resolver_ids, &["earthdata_https"]);
    assert_eq!(profile_for_id(profile.profile_id), Some(profile));

    let burst = profile_for_dataset("SLC-BURST").expect("Burst profile");
    assert_eq!(burst.delivery_model, AsfDeliveryModel::MultiAssetProduct);
    assert_eq!(burst.approved_resolver_ids, &["earthdata_https"]);

    let historical = profile_for_dataset("ERS").expect("ERS profile");
    assert_eq!(historical.delivery_model, AsfDeliveryModel::Unreviewed);
    assert!(historical.approved_resolver_ids.is_empty());
}

#[test]
fn every_enabled_asf_dataset_has_an_explicit_versioned_identity_profile() {
    let expected = [
        ("SENTINEL-1", "asf.sentinel-1.standard.v1", true),
        ("SLC-BURST", "asf.sentinel-1.burst.v1", false),
        ("ALOS PALSAR", "asf.alos-palsar.v2", false),
        ("ERS", "asf.ers.v1", true),
        ("JERS-1", "asf.jers-1.v1", true),
        ("RADARSAT-1", "asf.radarsat-1.v1", true),
        ("SEASAT", "asf.seasat.v1", true),
    ];
    for (dataset, rule_id, allows_scene_name) in expected {
        let profile = profile_for_dataset(dataset).expect(dataset);
        assert_eq!(profile.dataset, dataset);
        assert_eq!(profile.rule_id, rule_id);
        assert_eq!(profile.allow_scene_name_identity, allows_scene_name);
        assert!(!profile.item_id_fields.is_empty());
        assert!(!profile.asset_id_fields.is_empty());
        assert!(!profile.display_alias_fields.is_empty());
        assert!(!profile.critical_identity_fields.is_empty());
    }
}

#[test]
fn collection_identity_fixtures_preserve_products_assets_and_diagnostics() {
    for case in fixture().cases {
        let batch = map_and_aggregate_sar_features(&case.dataset, &case.features);
        let mut product_ids = batch
            .products
            .iter()
            .filter_map(|product| product.provider_product_id.clone())
            .collect::<Vec<_>>();
        product_ids.sort();
        let mut expected_ids = case.expected.provider_product_ids;
        expected_ids.sort();

        assert_eq!(product_ids, expected_ids, "{}", case.name);
        assert_eq!(
            batch.products.len(),
            case.expected.product_count,
            "{}",
            case.name
        );
        assert_eq!(
            batch.asset_count, case.expected.asset_count,
            "{}",
            case.name
        );
        assert_eq!(
            batch.replayed_record_count, case.expected.replayed_record_count,
            "{}",
            case.name
        );
        assert_eq!(
            batch.mapping_error_count, case.expected.mapping_error_count,
            "{}",
            case.name
        );
        assert_eq!(
            batch.identity_conflict_count, case.expected.identity_conflict_count,
            "{}",
            case.name
        );
        for product in &batch.products {
            assert_eq!(
                product.provider_product_key.as_ref(),
                Some(&product.product_key),
                "{}",
                case.name
            );
            assert!(product.identity_rule_id.is_some(), "{}", case.name);
            for asset in &product.assets {
                assert!(asset.asset_key.is_some(), "{}", case.name);
                assert!(asset.stable_asset_key.is_some(), "{}", case.name);
            }
        }
    }
}
