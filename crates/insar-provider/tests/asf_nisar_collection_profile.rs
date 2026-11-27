use std::{fs, path::PathBuf};

use insar_core::AssetRole;
use insar_provider::catalog::{
    CatalogDeliveryModel, CollectionProfileRegistry, NisarRslcProductSpecificationProfileRegistry,
    NISAR_BAND, NISAR_COLLECTION_SHORT_NAME, NISAR_COLLECTION_VERSION, NISAR_DATASET_SELECTOR,
    NISAR_FILENAME_RULE_ID, NISAR_FLIGHT_DIRECTIONS, NISAR_FRAME_COVERAGES,
    NISAR_FREQUENCY_A_POLARIZATIONS, NISAR_FREQUENCY_B_POLARIZATIONS, NISAR_IDENTITY_RULE_ID,
    NISAR_LOGICAL_DATASET, NISAR_LOOK_DIRECTIONS, NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_CRID,
    NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, NISAR_PRODUCTION_CONFIGURATION,
    NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
    NISAR_QUALITY_STATUS, NISAR_RANGE_BANDWIDTHS, NISAR_REQUIRED_PRODUCT_TYPE,
    NISAR_RESOLVER_POLICY_ID, NISAR_RESOLVER_POLICY_ID_V1, NISAR_RESOLVER_POLICY_ID_V2,
    NISAR_SCIENCE_LEVEL,
};
use serde_json::Value;

fn profile_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1/collection-profile-valid.json");
    serde_json::from_str(&fs::read_to_string(path).expect("NISAR fixture exists"))
        .expect("NISAR fixture is valid JSON")
}

#[test]
fn nisar_profile_is_exact_production_enabled_and_archive_first() {
    let fixture = profile_fixture();
    assert_eq!(fixture["status"], "accepted_production_enabled");
    let expected = &fixture["profile"];
    let registry = CollectionProfileRegistry;
    let profile = registry
        .get(NISAR_PROFILE_ID)
        .expect("the exact NISAR Provisional RSLC profile is registered");

    assert_eq!(profile.profile_id, expected["collection_profile_id"]);
    assert_eq!(profile.logical_dataset, expected["logical_dataset"]);
    assert_eq!(profile.provider_id, expected["provider_id"]);
    assert_eq!(
        profile.native_collection_id,
        expected["native_collection_id"]
    );
    assert_eq!(profile.supplemental_provider_id, "none");
    assert_eq!(profile.supplemental_collection_id, "none");
    assert_eq!(profile.required_product_type, "RSLC");
    assert_eq!(profile.required_product_type, NISAR_REQUIRED_PRODUCT_TYPE);
    assert_eq!(profile.identity_rule_id, NISAR_IDENTITY_RULE_ID);
    assert_eq!(profile.filename_rule_id, NISAR_FILENAME_RULE_ID);
    assert_eq!(profile.resolver_policy_id, NISAR_RESOLVER_POLICY_ID);
    assert_eq!(profile.delivery_model, CatalogDeliveryModel::ArchiveFirst);
    assert!(profile.production_enabled);

    assert_eq!(NISAR_PROVIDER_ID, expected["provider_id"]);
    assert_eq!(NISAR_DATASET_SELECTOR, expected["dataset_selector"]);
    assert_eq!(
        NISAR_COLLECTION_SHORT_NAME,
        expected["collection_short_name"]
    );
    assert_eq!(NISAR_COLLECTION_VERSION, expected["collection_version"]);
    assert_eq!(NISAR_SCIENCE_LEVEL, expected["science_level"]);
    assert_eq!(NISAR_QUALITY_STATUS, expected["quality_status"]);
    assert_eq!(
        NISAR_PRODUCTION_CONFIGURATION,
        expected["production_configuration"]
    );
    assert_eq!(NISAR_BAND, expected["band"]);
    assert_eq!(expected["primary_delivery"], "complete_hdf5");
    assert_eq!(expected["frequency_scope"], "frequency_a_or_b_required");

    assert_eq!(NISAR_FRAME_COVERAGES, &["FULL", "PARTIAL"]);
    assert_eq!(
        NISAR_FREQUENCY_A_POLARIZATIONS,
        &["HH", "HH+HV", "VV+VH", "HH+HV+VH+VV"]
    );
    assert_eq!(
        NISAR_FREQUENCY_B_POLARIZATIONS,
        &["HH", "HH+HV", "VV", "VV+VH"]
    );
    assert_eq!(NISAR_RANGE_BANDWIDTHS, &["5", "20+5", "40+5", "77"]);
    assert_eq!(NISAR_FLIGHT_DIRECTIONS, &["ASCENDING", "DESCENDING"]);
    assert_eq!(NISAR_LOOK_DIRECTIONS, &["LEFT", "RIGHT"]);

    assert_eq!(
        profile.normalized_role_for_native_roles(&["archive".to_owned()]),
        None
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["primary".to_owned()]),
        None
    );
    assert_ne!(
        profile.normalized_role_for_native_roles(&["archive".to_owned()]),
        Some(AssetRole::Primary)
    );
    assert_eq!(
        registry
            .for_logical_dataset(NISAR_LOGICAL_DATASET)
            .map(|value| value.profile_id),
        Some(NISAR_PROFILE_ID)
    );
}

#[test]
fn nisar_profile_has_no_beta_or_product_family_aliases() {
    let registry = CollectionProfileRegistry;

    assert_eq!(NISAR_NATIVE_COLLECTION_ID, "C2853145197-ASF");
    assert!(registry.get("asf.nisar.l.rslc.beta.v1").is_none());
    assert!(registry
        .for_logical_dataset("sar.nisar.l.rslc.beta.v1")
        .is_none());
    assert!(registry
        .for_logical_dataset(NISAR_COLLECTION_SHORT_NAME)
        .is_none());
    for family in [
        "rifg", "runw", "roff", "gslc", "gcov", "gunw", "goff", "l0b", "sme2",
    ] {
        assert!(registry
            .for_logical_dataset(&format!("sar.nisar.l.{family}.provisional.v1"))
            .is_none());
    }
}

#[test]
fn p05023_product_specification_profile_is_bounded_versioned_and_v2_bound() {
    let registry = NisarRslcProductSpecificationProfileRegistry;
    let profile = registry
        .for_crid(NISAR_P05023_CRID)
        .expect("the reviewed Provisional CRID is registered");

    assert_eq!(NISAR_P05023_CRID, "P05023");
    assert_eq!(NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, "1.5.0");
    assert_eq!(NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION, 1);
    assert_eq!(registry.version(), 1);
    assert_eq!(profile.registry_version, registry.version());
    assert_eq!(profile.crid, NISAR_P05023_CRID);
    assert_eq!(
        profile.product_specification_version,
        NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
    );
    assert_eq!(profile.resolver_policy_id, NISAR_RESOLVER_POLICY_ID_V2);
    assert_eq!(NISAR_RESOLVER_POLICY_ID, NISAR_RESOLVER_POLICY_ID_V2);
    assert_eq!(
        NISAR_RESOLVER_POLICY_ID_V1,
        "asf.nisar.earthdata-complete-hdf5.v1"
    );
    assert_eq!(
        NISAR_RESOLVER_POLICY_ID_V2,
        "asf.nisar.earthdata-complete-hdf5.v2"
    );
    assert!(registry.for_crid("").is_none());
    assert!(registry.for_crid("P05022").is_none());
    assert!(registry.for_crid("p05023").is_none());
}
