use std::{fs, path::PathBuf};

use insar_core::AssetRole;
use insar_provider::catalog::{
    alos2_expected_polarizations, CatalogDeliveryModel, CollectionProfileRegistry,
    ALOS2_COLLECTION_SHORT_NAME, ALOS2_COLLECTION_VERSION, ALOS2_LOGICAL_DATASET,
    ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROCESSING_LEVELS, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    ALOS2_SCAN_SAR_BEAM_MODES,
};
use serde_json::Value;

fn profile_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1/collection-profile-valid.json");
    serde_json::from_str(&fs::read_to_string(path).expect("ALOS-2 fixture exists"))
        .expect("ALOS-2 fixture is valid JSON")
}

#[test]
fn alos2_profile_is_exact_production_enabled_and_archive_first() {
    let expected = profile_fixture();
    let expected = &expected["profile"];
    let registry = CollectionProfileRegistry;
    let profile = registry
        .get(ALOS2_PROFILE_ID)
        .expect("the exact ALOS-2 profile is registered");

    assert_eq!(profile.profile_id, expected["collection_profile_id"]);
    assert_eq!(profile.logical_dataset, expected["logical_dataset"]);
    assert_eq!(profile.provider_id, expected["provider_id"]);
    assert_eq!(
        profile.native_collection_id,
        expected["native_collection_id"]
    );
    assert_eq!(profile.required_product_type, ALOS2_COLLECTION_SHORT_NAME);
    assert_eq!(profile.delivery_model, CatalogDeliveryModel::ArchiveFirst);
    assert!(profile.production_enabled);
    assert_eq!(ALOS2_COLLECTION_VERSION, expected["collection_version"]);
    assert_eq!(ALOS2_PROCESSING_LEVELS, &["1.1"]);
    assert_eq!(ALOS2_SCAN_SAR_BEAM_MODES, &["WBS", "WBD"]);
    assert_eq!(alos2_expected_polarizations("WBS"), Some(&["HH"][..]));
    assert_eq!(alos2_expected_polarizations("WBD"), Some(&["HH", "HV"][..]));
    assert_eq!(alos2_expected_polarizations("FBS"), None);
    assert_eq!(
        profile.normalized_role_for_native_roles(&["archive".to_owned()]),
        Some(AssetRole::Primary)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["data".to_owned()]),
        None
    );
    assert_eq!(
        registry
            .for_logical_dataset(ALOS2_LOGICAL_DATASET)
            .map(|value| value.profile_id),
        Some(ALOS2_PROFILE_ID)
    );
}

#[test]
fn historical_alos_profile_and_collection_are_not_alos2_aliases() {
    let registry = CollectionProfileRegistry;

    assert_eq!(ALOS2_PROVIDER_ID, "asf.nasa.earthdata");
    assert_eq!(ALOS2_NATIVE_COLLECTION_ID, "C3315903479-ASF");
    assert_eq!(ALOS2_COLLECTION_SHORT_NAME, "ALOS2_L1_PSR2");
    assert!(registry.get("asf.alos-palsar.v2").is_none());
    assert!(registry.for_logical_dataset("ALOS PALSAR").is_none());
    assert!(registry.for_logical_dataset("ALOS2_L1_PSR2").is_none());
}
