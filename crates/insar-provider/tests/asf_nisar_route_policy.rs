use insar_provider::catalog::{
    DatasetRoutePolicyError, DatasetRoutePolicyRegistry, RouteUse, NISAR_LOGICAL_DATASET,
    NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_ROUTE_ID,
};

#[cfg(feature = "nisar-subset-live-acceptance")]
use insar_provider::catalog::{
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROVIDER_ID,
};

const NISAR_SUPPORTED_FILTER_KEYS: &[&str] = &[
    "datetime",
    "production_configuration",
    "frame_coverage",
    "frequency_a_polarization",
    "frequency_b_polarization",
    "range_bandwidth",
    "joint_observation",
    "flight_direction",
    "look_direction",
    "track",
    "relative_orbit",
    "frame",
];

#[test]
fn nisar_route_is_exact_production_enabled_and_filter_closed() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve(
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        )
        .expect("the exact NISAR offline route resolves");

    assert_eq!(NISAR_ROUTE_ID, "asf.nisar.l.rslc.provisional.route.v1");
    assert_eq!(route.route_id, NISAR_ROUTE_ID);
    assert_eq!(route.collection_profile_id, NISAR_PROFILE_ID);
    assert_eq!(route.catalogue_aoi_crs, "EPSG:4326");
    assert!(route.production_enabled);
    assert_eq!(route.supported_filter_keys, NISAR_SUPPORTED_FILTER_KEYS);
    registry
        .validate_filter_keys(NISAR_LOGICAL_DATASET, NISAR_SUPPORTED_FILTER_KEYS)
        .expect("the closed evidence-backed NISAR filter set is accepted");
    registry
        .resolve(
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
            RouteUse::Production,
        )
        .expect("the production NISAR route resolves");
}

#[test]
fn nisar_route_rejects_beta_aliases_and_unknown_filters() {
    let registry = DatasetRoutePolicyRegistry;

    assert!(matches!(
        registry.resolve(
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            "C2850225585-ASF",
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
    ));
    assert!(matches!(
        registry.resolve(
            NISAR_LOGICAL_DATASET,
            "asf.alias",
            NISAR_NATIVE_COLLECTION_ID,
            RouteUse::OfflineFixture,
        ),
        Err(DatasetRoutePolicyError::ProviderMismatch { .. })
    ));
    for logical_dataset in [
        "sar.nisar.l.rslc.beta.v1",
        "NISAR_L1_RSLC_PROVISIONAL_V1",
        "sar.nisar.s.rslc.provisional.v1",
    ] {
        assert!(matches!(
            registry.resolve(
                logical_dataset,
                NISAR_PROVIDER_ID,
                NISAR_NATIVE_COLLECTION_ID,
                RouteUse::OfflineFixture,
            ),
            Err(DatasetRoutePolicyError::UnknownLogicalDataset(_))
        ));
    }
    for unsupported in [
        "aoi",
        "page_size",
        "processing_level",
        "quality_status",
        "science_level",
        "band",
        "path",
        "s_band",
        "remote_subset",
    ] {
        assert_eq!(
            registry
                .validate_filter_keys(NISAR_LOGICAL_DATASET, &[unsupported])
                .unwrap_err(),
            DatasetRoutePolicyError::UnsupportedFilter(unsupported.to_owned())
        );
    }
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn controlled_live_candidate_is_nisar_only_and_production_enabled() {
    let registry = DatasetRoutePolicyRegistry;
    let route = registry
        .resolve_nisar_live_acceptance_candidate(
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
        )
        .expect("the exact NISAR controlled candidate resolves");
    assert_eq!(route.route_id, NISAR_ROUTE_ID);
    assert!(route.production_enabled);

    assert!(matches!(
        registry.resolve_nisar_live_acceptance_candidate(
            NISAR_LOGICAL_DATASET,
            "wrong.provider",
            NISAR_NATIVE_COLLECTION_ID,
        ),
        Err(DatasetRoutePolicyError::ProviderMismatch { .. })
    ));
    assert!(matches!(
        registry.resolve_nisar_live_acceptance_candidate(
            NISAR_LOGICAL_DATASET,
            NISAR_PROVIDER_ID,
            "wrong-collection",
        ),
        Err(DatasetRoutePolicyError::NativeCollectionMismatch { .. })
    ));
    assert!(matches!(
        registry.resolve_nisar_live_acceptance_candidate(
            ALOS2_LOGICAL_DATASET,
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
        ),
        Err(DatasetRoutePolicyError::ProductionDisabled(_))
    ));
}
