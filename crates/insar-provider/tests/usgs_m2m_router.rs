use std::sync::Arc;
use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use insar_provider::catalog::{
    DatasetRoutePolicyRegistry, RouteUse, LANDSAT_C2_L2_LOGICAL_DATASET,
    LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID,
};
use insar_provider::usgs_m2m::{
    landsat_scene_search_live_accepted, UsgsM2mCapability, UsgsM2mDatasetDiscoveryClient,
    UsgsM2mDatasetDiscoveryRouteRequest, UsgsM2mDatasetDiscoveryRouter, UsgsM2mDiscoveryState,
    UsgsM2mDiscoveryUse, UsgsM2mErrorCode, UsgsM2mSession, USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY,
    USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE, USGS_M2M_LANDSAT_C2_L2_UPSTREAM_IDENTITY,
};

#[test]
fn live_accepted_discovery_route_is_separate_from_scene_catalogue() {
    let session = Arc::new(
        UsgsM2mSession::new(ProviderNetworkPolicy::system(), Duration::from_secs(1))
            .expect("fixed discovery construction is lazy"),
    );
    let router =
        UsgsM2mDatasetDiscoveryRouter::new(Arc::new(UsgsM2mDatasetDiscoveryClient::new(session)));
    let request = UsgsM2mDatasetDiscoveryRouteRequest::landsat_c2_l2();
    let report = router.capability_report();

    assert_eq!(report.logical_dataset(), LANDSAT_C2_L2_LOGICAL_DATASET);
    assert_eq!(report.provider_id(), LANDSAT_C2_L2_PROVIDER_ID);
    assert_eq!(report.network_policy(), ProviderNetworkPolicy::system());
    let identity = USGS_M2M_LANDSAT_C2_L2_LIVE_IDENTITY.expect("live identity is closed");
    assert_eq!(report.state(), UsgsM2mDiscoveryState::LiveAccepted);
    assert_eq!(report.state().as_str(), "live_accepted");
    assert_eq!(report.live_identity(), Some(identity));
    assert_eq!(report.dataset_id(), Some(identity.dataset_id()));
    assert_eq!(
        report.upstream_dataset_identity(),
        Some(identity.dataset_alias())
    );
    assert_eq!(
        USGS_M2M_LANDSAT_C2_L2_UPSTREAM_IDENTITY,
        Some(identity.dataset_alias())
    );
    assert!(report.production_enabled());
    assert_eq!(identity.dataset_id(), "5e83d14f2fc39685");
    assert_eq!(identity.dataset_alias(), "landsat_ot_c2_l2");
    assert_eq!(identity.collection_name(), "Landsat 8-9 OLI/TIRS C2 L2");
    assert_eq!(
        identity.dataset_category_name(),
        "Landsat Collection 2 Level-2"
    );
    assert_eq!(identity.data_owner(), "LSAA");
    assert_eq!(identity.catalogs(), &["EE"]);
    let scene_acceptance = USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE
        .expect("live scene-search acceptance is closed");
    assert!(landsat_scene_search_live_accepted());
    assert_eq!(
        scene_acceptance.acceptance_id(),
        "LANDSAT-P4A-2A-LIVE-2026-07-29"
    );
    assert_eq!(scene_acceptance.dataset_identity(), identity);
    assert_eq!(
        report.capabilities(),
        &[
            UsgsM2mCapability::DatasetDiscovery,
            UsgsM2mCapability::RuntimeSession,
            UsgsM2mCapability::NetworkPolicy,
        ]
    );
    assert_eq!(
        router
            .resolve(&request, UsgsM2mDiscoveryUse::OfflineValidation)
            .unwrap_err()
            .code(),
        UsgsM2mErrorCode::ConfigInvalid
    );
    let route = router
        .resolve(&request, UsgsM2mDiscoveryUse::Production)
        .unwrap();
    assert_eq!(route.state(), UsgsM2mDiscoveryState::LiveAccepted);
    assert_eq!(route.dataset_id(), Some(identity.dataset_id()));
    assert_eq!(
        route.upstream_dataset_identity(),
        Some(identity.dataset_alias())
    );
    assert!(route.production_enabled());

    assert!(DatasetRoutePolicyRegistry
        .resolve(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            "fixture_l2_alias",
            RouteUse::Production,
        )
        .is_err());
    assert!(DatasetRoutePolicyRegistry
        .resolve(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            RouteUse::Production,
        )
        .is_ok());
}
