use insar_core::AssetRole;
use insar_provider::catalog::{
    CatalogDeliveryModel, CollectionProfileRegistry, DatasetRoutePolicyRegistry,
    LandsatDeliveryOption, LandsatPreviewStrategy, LandsatProcessingLevel, LandsatSearchFilter,
    LandsatSessionBoundary, LandsatSpacecraft, RouteUse, BHOONIDHI_FILENAME_RULE_ID,
    BHOONIDHI_IDENTITY_RULE_ID, BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID,
    BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_REQUIRED_PRODUCT_TYPE,
    BHOONIDHI_RESOLVER_POLICY_ID, LANDSAT_C2_L2_FOUNDATION_PROFILE_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID,
    LANDSAT_C2_L2_SCENE_PROFILE_ID, SENTINEL2_L1C_PROFILE_ID, SENTINEL2_L2A_PROFILE_ID,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2, USGS_M2M_SESSION_PROFILE_ID,
};

#[test]
fn sentinel2_profile_keeps_closed_roles_and_is_production_enabled() {
    let profiles = CollectionProfileRegistry;
    let profile = profiles
        .get(SENTINEL2_L2A_PROFILE_ID)
        .expect("reviewed Sentinel-2 profile exists");

    assert_eq!(profile.logical_dataset, "optical.sentinel-2.l2a");
    assert_eq!(profile.provider_id, "copernicus.dataspace.odata");
    assert_eq!(profile.native_collection_id, "SENTINEL-2");
    assert_eq!(
        profile.supplemental_provider_id,
        "copernicus.dataspace.stac"
    );
    assert_eq!(profile.supplemental_collection_id, "sentinel-2-l2a");
    assert_eq!(profile.required_product_type, "S2MSI2A");
    assert_eq!(profile.delivery_model, CatalogDeliveryModel::ArchiveFirst);
    assert!(profile.production_enabled);
    assert_eq!(
        profile.normalized_role_for_native_roles(&["thumbnail".to_string()]),
        Some(AssetRole::Thumbnail)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["overview".to_string()]),
        Some(AssetRole::Browse)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["archive".to_string()]),
        Some(AssetRole::Primary)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&[
            "unreviewed-native-role".to_string(),
            "overview".to_string(),
            "thumbnail".to_string(),
        ]),
        None
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["unreviewed-native-role".to_string()]),
        None
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["data".to_string()]),
        None
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["metadata".to_string()]),
        None
    );
}

#[test]
fn sentinel2_l1c_profile_is_exact_and_default_closed() {
    let profiles = CollectionProfileRegistry;
    let profile = profiles
        .get(SENTINEL2_L1C_PROFILE_ID)
        .expect("reviewed Sentinel-2 L1C foundation exists");

    assert_eq!(profile.logical_dataset, "optical.sentinel-2.l1c");
    assert_eq!(profile.provider_id, "copernicus.dataspace.odata");
    assert_eq!(profile.native_collection_id, "SENTINEL-2");
    assert_eq!(
        profile.supplemental_provider_id,
        "copernicus.dataspace.stac"
    );
    assert_eq!(profile.supplemental_collection_id, "sentinel-2-l1c");
    assert_eq!(profile.required_product_type, "S2MSI1C");
    assert_eq!(profile.identity_rule_id, "cdse.sentinel-2.l1c.identity.v1");
    assert_eq!(
        profile.filename_rule_id,
        "cdse.sentinel-2.l1c.archive-filename.v1"
    );
    assert_eq!(
        profile.resolver_policy_id,
        "cdse.sentinel-2.l1c.archive-resolver-policy.v1"
    );
    assert_eq!(profile.delivery_model, CatalogDeliveryModel::ArchiveFirst);
    assert!(profile.production_enabled);
    assert_eq!(
        profile.normalized_role_for_native_roles(&["archive".to_owned()]),
        Some(AssetRole::Primary)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["thumbnail".to_owned()]),
        Some(AssetRole::Thumbnail)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["overview".to_owned()]),
        Some(AssetRole::Browse)
    );
    assert_eq!(
        profiles
            .for_logical_dataset("optical.sentinel-2.l1c")
            .map(|value| value.profile_id),
        Some(SENTINEL2_L1C_PROFILE_ID)
    );
    assert!(profiles
        .deferred_fixture_reservations()
        .iter()
        .all(|reservation| reservation.logical_dataset != "optical.sentinel-2.l1c"));
}

#[test]
fn exact_source_lookup_is_generic_across_registered_products() {
    let profiles = CollectionProfileRegistry;

    let sentinel2 = profiles
        .for_source(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
        )
        .expect("Sentinel-2 resolves through the generic exact-source index");
    assert_eq!(sentinel2.profile_id, SENTINEL2_L2A_PROFILE_ID);

    let bhoonidhi = profiles
        .for_source(
            BHOONIDHI_LOGICAL_DATASET,
            BHOONIDHI_PROVIDER_ID,
            BHOONIDHI_NATIVE_COLLECTION_ID,
        )
        .expect("Bhoonidhi resolves through the same exact-source index");
    assert_eq!(bhoonidhi.profile_id, BHOONIDHI_PROFILE_ID);

    assert!(profiles
        .for_source(
            "optical.sentinel-2.l2a",
            BHOONIDHI_PROVIDER_ID,
            "SENTINEL-2",
        )
        .is_none());
    assert!(profiles
        .for_source(
            BHOONIDHI_LOGICAL_DATASET,
            BHOONIDHI_PROVIDER_ID,
            "SENTINEL-2",
        )
        .is_none());
}

#[test]
fn deferred_collections_remain_fixture_reservations_without_routes_or_profiles() {
    let profiles = CollectionProfileRegistry;
    for logical_dataset in [
        "optical.sentinel-3.olci",
        "optical.sentinel-3.slstr",
        "optical.hls-s30",
        "optical.hls-l30",
        "optical.modis",
        "optical.viirs",
        "optical.aster",
    ] {
        assert!(profiles.for_logical_dataset(logical_dataset).is_none());
    }
    assert_eq!(profiles.deferred_fixture_reservations().len(), 7);
}

#[test]
fn landsat_c2_l2_foundation_is_closed_and_credential_isolated() {
    let profiles = CollectionProfileRegistry;
    let foundation = profiles.landsat_c2_l2_foundation();

    assert_eq!(foundation.profile_id, LANDSAT_C2_L2_FOUNDATION_PROFILE_ID);
    assert_eq!(foundation.logical_dataset, LANDSAT_C2_L2_LOGICAL_DATASET);
    assert_eq!(foundation.provider_id, LANDSAT_C2_L2_PROVIDER_ID);
    assert_eq!(foundation.upstream_dataset_id, None);
    assert_eq!(foundation.session_boundary, LandsatSessionBoundary::UsgsM2m);
    assert_eq!(
        foundation.session_boundary.profile_id(),
        USGS_M2M_SESSION_PROFILE_ID
    );
    assert!(foundation
        .session_boundary
        .matches_profile(USGS_M2M_SESSION_PROFILE_ID));
    assert!(!foundation
        .session_boundary
        .matches_profile("copernicus-dataspace-default"));
    assert!(!foundation
        .session_boundary
        .matches_profile("earthdata-default"));
    assert!(!foundation.is_executable());
    assert!(!foundation.permits_runtime_credential_binding());
    assert_eq!(
        foundation.supported_filters,
        &[
            LandsatSearchFilter::Aoi,
            LandsatSearchFilter::AcquisitionDateRange,
            LandsatSearchFilter::CloudCoverLte,
            LandsatSearchFilter::Spacecraft,
            LandsatSearchFilter::ProcessingLevel,
        ]
    );
    assert_eq!(
        foundation.spacecraft,
        &[LandsatSpacecraft::Landsat8, LandsatSpacecraft::Landsat9]
    );
    assert_eq!(
        foundation.processing_levels,
        &[LandsatProcessingLevel::L2Sp, LandsatProcessingLevel::L2Sr]
    );
    assert_eq!(
        foundation.delivery_options,
        &[LandsatDeliveryOption::FullBundle]
    );
    assert_eq!(
        foundation.preview_strategy,
        LandsatPreviewStrategy::ExactSceneBrowse
    );
    let scene_profile = profiles
        .for_logical_dataset(LANDSAT_C2_L2_LOGICAL_DATASET)
        .expect("P4A-2A registers one scene-only Landsat profile");
    assert_eq!(scene_profile.profile_id, LANDSAT_C2_L2_SCENE_PROFILE_ID);
    assert_eq!(
        scene_profile.native_collection_id,
        LANDSAT_C2_L2_NATIVE_COLLECTION_ID
    );
    assert_eq!(scene_profile.provider_id, LANDSAT_C2_L2_PROVIDER_ID);
    assert!(scene_profile.production_enabled);
    assert_eq!(
        scene_profile.normalized_role_for_native_roles(&["full_bundle".to_owned()]),
        Some(insar_core::AssetRole::Primary)
    );
    assert_eq!(
        scene_profile.normalized_role_for_native_roles(&["selected_band".to_owned()]),
        None
    );
    assert_eq!(scene_profile.supplemental_provider_id, "none");
    assert_eq!(
        scene_profile.resolver_policy_id,
        USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2
    );
    assert!(DatasetRoutePolicyRegistry
        .resolve(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            RouteUse::Production,
        )
        .is_ok());
}

#[test]
fn bhoonidhi_ssar_profile_is_exact_and_keeps_science_asset_roles_closed() {
    let profiles = CollectionProfileRegistry;
    let profile = profiles
        .get(BHOONIDHI_PROFILE_ID)
        .expect("Bhoonidhi S-RSLC profile is registered");

    assert_eq!(profile.logical_dataset, BHOONIDHI_LOGICAL_DATASET);
    assert_eq!(profile.provider_id, BHOONIDHI_PROVIDER_ID);
    assert_eq!(profile.native_collection_id, BHOONIDHI_NATIVE_COLLECTION_ID);
    assert_eq!(profile.supplemental_provider_id, "none");
    assert_eq!(profile.supplemental_collection_id, "none");
    assert_eq!(
        profile.required_product_type,
        BHOONIDHI_REQUIRED_PRODUCT_TYPE
    );
    assert_eq!(profile.identity_rule_id, BHOONIDHI_IDENTITY_RULE_ID);
    assert_eq!(profile.filename_rule_id, BHOONIDHI_FILENAME_RULE_ID);
    assert_eq!(profile.resolver_policy_id, BHOONIDHI_RESOLVER_POLICY_ID);
    assert_eq!(profile.delivery_model, CatalogDeliveryModel::ArchiveFirst);
    assert!(profile.production_enabled);
    assert_eq!(
        profile.normalized_role_for_native_roles(&["metadata".to_owned()]),
        Some(AssetRole::Metadata)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["thumbnail".to_owned()]),
        Some(AssetRole::Thumbnail)
    );
    assert_eq!(
        profile.normalized_role_for_native_roles(&["data".to_owned()]),
        None
    );
    assert_eq!(
        profiles
            .for_logical_dataset(BHOONIDHI_LOGICAL_DATASET)
            .map(|value| value.profile_id),
        Some(BHOONIDHI_PROFILE_ID)
    );
}
