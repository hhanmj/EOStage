use insar_core::provider_product_key;
use insar_provider::catalog::{
    CollectionProfileRegistry, EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    EARTH_SEARCH_LANDSAT_PROVIDER_ID, EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
    EARTH_SEARCH_LANDSAT_SEARCH_ONLY_POLICY_ID, LANDSAT_C2_L2_LOGICAL_DATASET,
    LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID, MPC_LANDSAT_SCENE_PROFILE_ID,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V1,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2,
};

#[test]
fn exact_source_profiles_share_science_but_not_provider_identity() {
    let registry = CollectionProfileRegistry;
    let usgs = registry
        .for_source(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            LANDSAT_C2_L2_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
        )
        .unwrap();
    let microsoft = registry
        .for_source(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            MPC_LANDSAT_PROVIDER_ID,
            MPC_LANDSAT_COLLECTION_ID,
        )
        .unwrap();
    let earth = registry
        .for_source(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            EARTH_SEARCH_LANDSAT_PROVIDER_ID,
            EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        )
        .unwrap();
    assert_eq!(usgs.profile_id, LANDSAT_C2_L2_SCENE_PROFILE_ID);
    assert_eq!(microsoft.profile_id, MPC_LANDSAT_SCENE_PROFILE_ID);
    assert_eq!(usgs.logical_dataset, microsoft.logical_dataset);
    assert_eq!(microsoft.logical_dataset, earth.logical_dataset);
    assert_eq!(usgs.required_product_type, microsoft.required_product_type);
    assert_eq!(microsoft.required_product_type, earth.required_product_type);
    assert_ne!(usgs.identity_rule_id, microsoft.identity_rule_id);
    assert_ne!(microsoft.identity_rule_id, earth.identity_rule_id);
    assert_ne!(usgs.resolver_policy_id, microsoft.resolver_policy_id);
    assert_eq!(earth.profile_id, EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID);
    assert_eq!(
        earth.resolver_policy_id,
        EARTH_SEARCH_LANDSAT_SEARCH_ONLY_POLICY_ID
    );
    assert_ne!(
        provider_product_key(usgs.provider_id, usgs.native_collection_id, "scene").unwrap(),
        provider_product_key(
            microsoft.provider_id,
            microsoft.native_collection_id,
            "scene"
        )
        .unwrap()
    );
    assert_ne!(
        provider_product_key(
            microsoft.provider_id,
            microsoft.native_collection_id,
            "scene"
        )
        .unwrap(),
        provider_product_key(earth.provider_id, earth.native_collection_id, "scene").unwrap()
    );
}

#[test]
fn source_profile_never_accepts_a_mixed_provider_collection_pair() {
    let registry = CollectionProfileRegistry;
    for (provider, collection) in [
        (MPC_LANDSAT_PROVIDER_ID, LANDSAT_C2_L2_NATIVE_COLLECTION_ID),
        (LANDSAT_C2_L2_PROVIDER_ID, MPC_LANDSAT_COLLECTION_ID),
        (
            EARTH_SEARCH_LANDSAT_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
        ),
        (
            LANDSAT_C2_L2_PROVIDER_ID,
            EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        ),
        ("unknown", MPC_LANDSAT_COLLECTION_ID),
    ] {
        assert!(registry
            .for_source(LANDSAT_C2_L2_LOGICAL_DATASET, provider, collection)
            .is_none());
    }
    assert!(registry
        .for_source(
            "optical.sentinel-2.l2a",
            MPC_LANDSAT_PROVIDER_ID,
            MPC_LANDSAT_COLLECTION_ID
        )
        .is_none());
}

#[test]
fn legacy_profile_projection_keeps_identity_ids_and_current_policy_is_v2() {
    let registry = CollectionProfileRegistry;
    let old = registry
        .for_logical_dataset(LANDSAT_C2_L2_LOGICAL_DATASET)
        .unwrap();
    assert_eq!(old.provider_id, "usgs.eros.m2m");
    assert_eq!(old.native_collection_id, "landsat_ot_c2_l2");
    assert_eq!(old.profile_id, "usgs.eros.m2m.landsat.c2.l2.scene.v1");
    assert_eq!(
        old.identity_rule_id,
        "usgs.eros.m2m.landsat.c2.l2.identity.v1"
    );
    assert_eq!(
        old.resolver_policy_id,
        USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2
    );
    assert_eq!(
        USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V1,
        "usgs.eros.m2m.landsat.c2.l2.full-bundle-resolver-policy.v1"
    );
}

#[test]
fn all_landsat_channels_resolve_exactly_without_a_list_order_default() {
    use insar_provider::catalog::{DatasetRoutePolicyRegistry, RouteUse};
    let routes = DatasetRoutePolicyRegistry;
    assert!(routes
        .for_logical_dataset(LANDSAT_C2_L2_LOGICAL_DATASET)
        .is_none());
    for (provider, collection, profile) in [
        (
            MPC_LANDSAT_PROVIDER_ID,
            MPC_LANDSAT_COLLECTION_ID,
            MPC_LANDSAT_SCENE_PROFILE_ID,
        ),
        (
            EARTH_SEARCH_LANDSAT_PROVIDER_ID,
            EARTH_SEARCH_LANDSAT_COLLECTION_ID,
            EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
        ),
        (
            LANDSAT_C2_L2_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            LANDSAT_C2_L2_SCENE_PROFILE_ID,
        ),
    ] {
        let found = routes
            .resolve(
                LANDSAT_C2_L2_LOGICAL_DATASET,
                provider,
                collection,
                RouteUse::Production,
            )
            .unwrap();
        assert_eq!(found.collection_profile_id, profile);
    }
    assert!(routes
        .resolve(
            LANDSAT_C2_L2_LOGICAL_DATASET,
            MPC_LANDSAT_PROVIDER_ID,
            LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
            RouteUse::Production
        )
        .is_err());
}
