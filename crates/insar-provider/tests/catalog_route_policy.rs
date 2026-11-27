use insar_provider::catalog::{
    CollectionProfileRegistry, DatasetRoutePolicy, DatasetRoutePolicyError,
    DatasetRoutePolicyRegistry, RouteUse, BHOONIDHI_LOGICAL_DATASET,
    BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_ROUTE_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID,
    LANDSAT_C2_L2_SCENE_ROUTE_ID, MPC_SENTINEL2_L2A_COLLECTION_ID, MPC_SENTINEL2_L2A_PROFILE_ID,
    MPC_SENTINEL2_L2A_PROVIDER_ID, MPC_SENTINEL2_L2A_ROUTE_ID, SENTINEL2_L1C_ROUTE_ID,
    SENTINEL2_L2A_ROUTE_ID,
};

#[test]
fn sentinel2_route_is_exact_and_landsat_scene_route_is_live_accepted() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::OfflineFixture,
        )
        .expect("the reviewed fixture route resolves");

    assert_eq!(route.route_id, SENTINEL2_L2A_ROUTE_ID);
    assert_eq!(
        route.collection_profile_id,
        "cdse.sentinel-2.l2a.collection.v1"
    );
    assert!(route.production_enabled);
    assert_eq!(route.catalogue_aoi_crs, "EPSG:4326");
    let microsoft = registry
        .resolve(
            "optical.sentinel-2.l2a",
            MPC_SENTINEL2_L2A_PROVIDER_ID,
            MPC_SENTINEL2_L2A_COLLECTION_ID,
            RouteUse::Production,
        )
        .expect("the reviewed Microsoft Sentinel-2 search-only route resolves exactly");
    assert_eq!(microsoft.route_id, MPC_SENTINEL2_L2A_ROUTE_ID);
    assert_eq!(
        microsoft.collection_profile_id,
        MPC_SENTINEL2_L2A_PROFILE_ID
    );
    assert!(microsoft.production_enabled);
    registry
        .validate_filter_keys("optical.sentinel-2.l2a", &["spacecraft"])
        .unwrap();
    assert!(route.supported_filter_keys.contains(&"spacecraft"));
    assert!(microsoft.supported_filter_keys.contains(&"spacecraft"));
    assert!(registry
        .validate_filter_keys("optical.sentinel-2.l2a", &["bands"])
        .is_err());
    assert!(registry
        .validate_filter_keys("optical.sentinel-2.l1c", &["spacecraft"])
        .is_ok());
    assert!(registry
        .validate_filter_keys("optical.sentinel-2.l1c", &["bands"])
        .is_err());
    assert_eq!(
        registry
            .resolve(
                "optical.sentinel-2.l2a",
                "copernicus.dataspace.odata",
                "SENTINEL-2",
                RouteUse::Production,
            )
            .expect("the reviewed Sentinel-2 route is production-enabled")
            .route_id,
        SENTINEL2_L2A_ROUTE_ID
    );

    let landsat = registry
        .resolve(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        )
        .expect("P4A-2A Landsat scene route is available for offline proof");
    assert_eq!(landsat.route_id, LANDSAT_C2_L2_SCENE_ROUTE_ID);
    assert!(landsat.production_enabled);
    assert_eq!(
        registry
            .resolve(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                LANDSAT_C2_L2_PROVIDER_ID,
                LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
                RouteUse::Production,
            )
            .expect("the live-accepted Landsat scene route resolves for production")
            .route_id,
        LANDSAT_C2_L2_SCENE_ROUTE_ID
    );
}

#[test]
fn sentinel2_l1c_route_is_exact_and_production_enabled() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve(
            "optical.sentinel-2.l1c",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::OfflineFixture,
        )
        .expect("the reviewed L1C foundation route resolves offline");

    assert_eq!(route.route_id, SENTINEL2_L1C_ROUTE_ID);
    assert_eq!(
        route.collection_profile_id,
        "cdse.sentinel-2.l1c.collection.v1"
    );
    assert!(route.production_enabled);
    assert_eq!(route.catalogue_aoi_crs, "EPSG:4326");
    let production = registry
        .resolve(
            "optical.sentinel-2.l1c",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::Production,
        )
        .expect("the reviewed L1C route resolves for production");
    assert_eq!(production.route_id, SENTINEL2_L1C_ROUTE_ID);
    assert!(production.production_enabled);
    registry
        .validate_filter_keys(
            "optical.sentinel-2.l1c",
            &["product_type", "cloud_cover_lte", "datetime", "spacecraft"],
        )
        .expect("the closed L1C filters are accepted");
    assert_eq!(
        registry
            .validate_filter_keys("optical.sentinel-2.l1c", &["processing_levels"])
            .unwrap_err(),
        DatasetRoutePolicyError::UnsupportedFilter("processing_levels".to_owned())
    );
}

#[test]
fn route_registry_rejects_unknown_duplicate_and_mismatched_routes() {
    let registry = DatasetRoutePolicyRegistry;
    assert!(matches!(
        registry.resolve(
            "missing.dataset",
            "fixture",
            "fixture",
            RouteUse::OfflineFixture
        ),
        Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))
    ));
    assert!(matches!(
        registry.resolve(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.stac",
            "sentinel-2-l2a",
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::ProviderUnavailable(_))
    ));
    assert!(matches!(
        registry.resolve(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "wrong-collection",
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
    ));

    let primary = DatasetRoutePolicy {
        route_id: "fixture.primary.route.v1",
        logical_dataset: "optical.sentinel-2.l2a",
        provider_id: "fixture.primary",
        native_collection_id: "fixture-primary",
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: "fixture.profile.v1",
        production_enabled: true,
        supported_filter_keys: &[],
    };
    let secondary = DatasetRoutePolicy {
        route_id: "fixture.secondary.route.v1",
        logical_dataset: "optical.sentinel-2.l2a",
        provider_id: "fixture.secondary",
        native_collection_id: "fixture-secondary",
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: "fixture.profile.v1",
        production_enabled: false,
        supported_filter_keys: &[],
    };
    DatasetRoutePolicyRegistry::validate_definitions(&[primary, secondary])
        .expect("one product may declare multiple distinct provider routes");
    let dual_routes = [primary, secondary];
    let resolved = DatasetRoutePolicyRegistry::resolve_from_definitions(
        &dual_routes,
        "optical.sentinel-2.l2a",
        "fixture.secondary",
        "fixture-secondary",
        RouteUse::OfflineFixture,
    )
    .expect("the exact secondary provider/collection route resolves");
    assert_eq!(resolved.route_id, secondary.route_id);
    assert!(matches!(
        DatasetRoutePolicyRegistry::resolve_from_definitions(
            &dual_routes,
            "optical.sentinel-2.l2a",
            "fixture.secondary",
            "fixture-secondary",
            RouteUse::Production,
        ),
        Err(DatasetRoutePolicyError::ProductionDisabled(
            "fixture.secondary.route.v1"
        ))
    ));
    assert!(matches!(
        DatasetRoutePolicyRegistry::resolve_from_definitions(
            &dual_routes,
            "optical.sentinel-2.l2a",
            "missing.provider",
            "missing-collection",
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::ProviderUnavailable(_))
    ));

    let duplicate_route_id = DatasetRoutePolicy {
        provider_id: "fixture.other",
        native_collection_id: "fixture-other",
        ..primary
    };
    assert!(matches!(
        DatasetRoutePolicyRegistry::validate_definitions(&[primary, duplicate_route_id]),
        Err(DatasetRoutePolicyError::DuplicateRouteId(_))
    ));
    let duplicate_binding = DatasetRoutePolicy {
        route_id: "fixture.duplicate-binding.route.v1",
        ..primary
    };
    assert!(matches!(
        DatasetRoutePolicyRegistry::validate_definitions(&[primary, duplicate_binding]),
        Err(DatasetRoutePolicyError::DuplicateProviderCollection { .. })
    ));

    for reservation in CollectionProfileRegistry.deferred_fixture_reservations() {
        assert!(matches!(
            registry.resolve(
                reservation.logical_dataset,
                "unreviewed.provider",
                "unreviewed-collection",
                RouteUse::Production,
            ),
            Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))
        ));
    }
}

#[test]
fn route_registry_rejects_unreviewed_filters_without_fallback() {
    let registry = DatasetRoutePolicyRegistry;
    registry
        .validate_filter_keys(
            "optical.sentinel-2.l2a",
            &["product_type", "cloud_cover_lte"],
        )
        .expect("reviewed fixture filters are accepted");
    registry
        .validate_filter_keys(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            &[
                "datetime",
                "cloud_cover_lte",
                "spacecraft",
                "processing_levels",
            ],
        )
        .expect("closed Landsat scene filters are accepted");
    assert_eq!(
        registry
            .validate_filter_keys("optical.sentinel-2.l2a", &["bbox"])
            .unwrap_err(),
        DatasetRoutePolicyError::UnsupportedFilter("bbox".to_string())
    );
    assert_eq!(
        registry
            .validate_filter_keys("optical.sentinel-2.l2a", &["invented_filter"])
            .unwrap_err(),
        DatasetRoutePolicyError::UnsupportedFilter("invented_filter".to_string())
    );
}

#[test]
fn bhoonidhi_ssar_route_requires_exact_identity_and_closed_filters() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve(
            BHOONIDHI_LOGICAL_DATASET,
            BHOONIDHI_PROVIDER_ID,
            BHOONIDHI_NATIVE_COLLECTION_ID,
            RouteUse::Production,
        )
        .expect("Bhoonidhi S-RSLC route is production-enabled");
    assert_eq!(route.route_id, BHOONIDHI_ROUTE_ID);
    assert_eq!(route.catalogue_aoi_crs, "EPSG:4326");
    assert!(route.production_enabled);
    registry
        .validate_filter_keys(
            BHOONIDHI_LOGICAL_DATASET,
            &[
                "datetime",
                "online",
                "mode",
                "polarization",
                "acquisition_mode",
                "product_code",
                "software_version",
                "producer",
                "platform",
                "instrument",
                "scene_identifier",
            ],
        )
        .expect("reviewed Bhoonidhi filters are accepted");
    assert_eq!(
        registry
            .validate_filter_keys(BHOONIDHI_LOGICAL_DATASET, &["product_type"])
            .unwrap_err(),
        DatasetRoutePolicyError::UnsupportedFilter("product_type".to_owned())
    );
    assert!(matches!(
        registry.resolve(
            BHOONIDHI_LOGICAL_DATASET,
            "asf.nasa.earthdata",
            BHOONIDHI_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::ProviderMismatch { .. })
    ));
}
