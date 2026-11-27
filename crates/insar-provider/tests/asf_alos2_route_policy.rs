use insar_provider::catalog::{
    DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse, ALOS2_LOGICAL_DATASET,
    ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID, ALOS2_ROUTE_ID,
};

#[test]
fn alos2_route_is_exact_production_enabled_and_filter_closed() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve(
            ALOS2_LOGICAL_DATASET,
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        )
        .expect("the exact ALOS-2 offline route resolves");

    assert_eq!(route.route_id, ALOS2_ROUTE_ID);
    assert_eq!(route.collection_profile_id, ALOS2_PROFILE_ID);
    assert_eq!(route.catalogue_aoi_crs, "EPSG:4326");
    assert!(route.production_enabled);
    assert_eq!(
        route.supported_filter_keys,
        &[
            "datetime",
            "beam_mode",
            "polarizations",
            "flight_direction",
            "look_direction",
            "path",
            "frame",
        ]
    );
    registry
        .resolve(
            ALOS2_LOGICAL_DATASET,
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
            RouteUse::Production,
        )
        .expect("the production ALOS-2 route resolves");
    registry
        .validate_filter_keys(
            ALOS2_LOGICAL_DATASET,
            &["datetime", "beam_mode", "polarizations", "path", "frame"],
        )
        .expect("reviewed ALOS-2 filter keys are accepted");
    assert_eq!(
        registry
            .validate_filter_keys(ALOS2_LOGICAL_DATASET, &["processing_levels"])
            .unwrap_err(),
        DatasetRoutePolicyError::UnsupportedFilter("processing_levels".to_owned())
    );
}

#[test]
fn historical_alos_identity_cannot_resolve_the_alos2_route() {
    let registry = DatasetRoutePolicyRegistry;

    assert!(matches!(
        registry.resolve(
            ALOS2_LOGICAL_DATASET,
            ALOS2_PROVIDER_ID,
            "C1206485527-ASF",
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
    ));
    assert!(matches!(
        registry.resolve(
            "ALOS PALSAR",
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))
    ));
}
