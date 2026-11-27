use insar_core::{ResourceDescriptor, ResourceRegistry, StaticCapabilityLifecycle};
use insar_provider::asf::collection_profile::{
    profile_for_dataset as asf_profile_for_dataset, profile_for_id as asf_profile_for_id,
};
use insar_provider::catalog::{
    CollectionProfileRegistry, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    NISAR_NATIVE_COLLECTION_ID, NISAR_P05023_PRODUCT_SPECIFICATION_VERSION, NISAR_PROFILE_ID,
    NISAR_PROVIDER_ID, SENTINEL2_L2A_PROFILE_ID,
};
use insar_provider::orbit_download::OrbitResolverEndpoint;
use insar_provider::planetary_computer::PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID;
use insar_provider::usgs_m2m::USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE;

fn registry() -> ResourceRegistry {
    ResourceRegistry::embedded().expect("embedded Resource Registry")
}

fn route<'a>(
    registry: &'a ResourceRegistry,
    resource_id: &str,
    route_id: &str,
) -> &'a insar_core::ProviderRouteDescriptor {
    registry
        .resource(resource_id)
        .expect("resource binding")
        .common()
        .provider_routes
        .iter()
        .find(|route| route.route_id == route_id)
        .expect("provider route binding")
}

#[test]
fn nisar_and_alos2_registry_routes_match_collection_profile_authority() {
    let registry = registry();
    let profiles = CollectionProfileRegistry;

    for (resource_id, route_id, profile_id, provider_id, native_collection_id) in [
        (
            "science.nisar.l.rslc.provisional.v1",
            "route.asf.nisar-rslc.catalog.v1",
            NISAR_PROFILE_ID,
            NISAR_PROVIDER_ID,
            NISAR_NATIVE_COLLECTION_ID,
        ),
        (
            "science.alos-2.palsar-2.scansar-l1.1.v1",
            "route.asf.alos2-scansar-l1.1.v1",
            ALOS2_PROFILE_ID,
            ALOS2_PROVIDER_ID,
            ALOS2_NATIVE_COLLECTION_ID,
        ),
    ] {
        let resource = registry.resource(resource_id).expect("science resource");
        let route = route(&registry, resource_id, route_id);
        let profile = profiles.get(profile_id).expect("collection profile");
        assert_eq!(
            resource.common().implementation_binding_id.as_deref(),
            Some(profile_id)
        );
        assert_eq!(route.implementation_binding_id, profile_id);
        assert_eq!(route.provider_id, provider_id);
        assert_eq!(
            route.native_collection_id.as_deref(),
            Some(native_collection_id)
        );
        assert_eq!(route.provider_id, profile.provider_id);
        assert_eq!(
            route.native_collection_id.as_deref(),
            Some(profile.native_collection_id)
        );
    }
}

#[test]
fn nisar_product_specification_version_matches_the_closed_crid_bridge() {
    let registry = registry();
    let resource = registry
        .resource("science.nisar.l.rslc.provisional.v1")
        .expect("NISAR resource");
    let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
        panic!("NISAR must be a science descriptor");
    };
    assert_eq!(science.product_variants.len(), 1);
    assert_eq!(
        science.product_variants[0].specification_version.as_deref(),
        Some(NISAR_P05023_PRODUCT_SPECIFICATION_VERSION)
    );
}

#[test]
fn sentinel1_standard_and_burst_bind_to_existing_asf_profiles() {
    let registry = registry();
    for (resource_id, route_id, profile_id, expected_dataset) in [
        (
            "science.sentinel-1.standard.v1",
            "route.asf.sentinel-1.standard.v1",
            "asf.sentinel-1.standard.v1",
            "SENTINEL-1",
        ),
        (
            "science.sentinel-1.slc-burst.v1",
            "route.asf.sentinel-1.slc-burst.v1",
            "asf.sentinel-1.burst.v1",
            "SLC-BURST",
        ),
    ] {
        let resource = registry.resource(resource_id).expect("Sentinel-1 resource");
        let profile = asf_profile_for_id(profile_id).expect("existing ASF profile");
        let route = route(&registry, resource_id, route_id);
        assert_eq!(
            resource.common().implementation_binding_id.as_deref(),
            Some(profile_id)
        );
        assert_eq!(route.implementation_binding_id, profile.profile_id);
        assert_eq!(
            route.native_collection_id.as_deref(),
            Some(expected_dataset)
        );
        assert_eq!(route.native_collection_id.as_deref(), Some(profile.dataset));
    }
}

#[test]
fn every_legacy_asf_selector_resolves_its_profile_through_resource_registry() {
    let registry = registry();
    // ERS, JERS-1 and RADARSAT-1 were removed from ASF's searchable
    // interfaces (Owner evidence 2026-10-31) and are retired: their selectors
    // no longer resolve to an implemented registry binding, but their
    // collection profiles stay registered so the provider code path can be
    // restored if ASF re-exposes the collections.
    for (selector, resource_id, profile_id) in [
        (
            "SENTINEL-1",
            "science.sentinel-1.standard.v1",
            "asf.sentinel-1.standard.v1",
        ),
        (
            "SLC-BURST",
            "science.sentinel-1.slc-burst.v1",
            "asf.sentinel-1.burst.v1",
        ),
        (
            "ALOS PALSAR",
            "science.alos.palsar.v1",
            "asf.alos-palsar.v2",
        ),
        ("SEASAT", "science.seasat.sar.v1", "asf.seasat.v1"),
    ] {
        let resource = registry
            .resource_for_provider_selector("asf.catalog", selector)
            .expect("selector is registered");
        assert_eq!(resource.common().resource_id, resource_id);
        assert_eq!(
            resource.common().implementation_binding_id.as_deref(),
            Some(profile_id)
        );
        let profile = asf_profile_for_dataset(selector).expect("profile is projected");
        assert_eq!(profile.profile_id, profile_id);
        assert_eq!(profile.dataset, selector);
    }

    for (selector, resource_id) in [
        ("ERS", "science.ers-1-2.sar.v1"),
        ("JERS-1", "science.jers-1.sar.v1"),
        ("RADARSAT-1", "science.radarsat-1.sar.v1"),
    ] {
        assert!(
            registry
                .resource_for_provider_selector("asf.catalog", selector)
                .is_none(),
            "{selector} is retired and no longer binds an implemented route"
        );
        let resource = registry.resource(resource_id).expect("retired resource");
        assert_eq!(
            resource.common().lifecycle,
            StaticCapabilityLifecycle::Retired
        );
        // The collection profile stays available for a future restore.
        assert!(asf_profile_for_dataset(selector).is_some());
    }
}

#[test]
fn sentinel2_l2a_and_landsat_routes_match_existing_provider_bindings() {
    let registry = registry();
    let profiles = CollectionProfileRegistry;

    let sentinel2 = registry
        .resource("science.sentinel-2.l2a.v1")
        .expect("Sentinel-2 L2A resource");
    let sentinel2_profile = profiles
        .get(SENTINEL2_L2A_PROFILE_ID)
        .expect("Sentinel-2 L2A profile");
    let sentinel2_route = route(
        &registry,
        "science.sentinel-2.l2a.v1",
        "route.cdse.sentinel-2.l2a.v1",
    );
    assert_eq!(
        sentinel2.common().implementation_binding_id.as_deref(),
        Some(SENTINEL2_L2A_PROFILE_ID)
    );
    assert_eq!(sentinel2_route.provider_id, sentinel2_profile.provider_id);
    assert_eq!(
        sentinel2_route.native_collection_id.as_deref(),
        Some(sentinel2_profile.native_collection_id)
    );

    let mpc = route(
        &registry,
        "science.landsat.c2-l2.v1",
        "route.mpc.landsat.c2-l2.v1",
    );
    assert_eq!(
        mpc.native_collection_id.as_deref(),
        Some(PLANETARY_COMPUTER_LANDSAT_COLLECTION_ID)
    );

    let usgs = route(
        &registry,
        "science.landsat.c2-l2.v1",
        "route.usgs.landsat.c2-l2.v1",
    );
    assert_eq!(usgs.provider_id, LANDSAT_C2_L2_PROVIDER_ID);
    assert_eq!(
        usgs.native_collection_id.as_deref(),
        Some(LANDSAT_C2_L2_NATIVE_COLLECTION_ID)
    );
    assert_eq!(
        usgs.implementation_binding_id,
        LANDSAT_C2_L2_SCENE_PROFILE_ID
    );
    assert_eq!(
        usgs.lifecycle,
        StaticCapabilityLifecycle::AdmissionControlled
    );

    let accepted = USGS_M2M_LANDSAT_C2_L2_SCENE_SEARCH_ACCEPTANCE
        .expect("current code carries accepted USGS scene-search evidence");
    assert_eq!(accepted.acceptance_id(), "LANDSAT-P4A-2A-LIVE-2026-07-29");
}

#[test]
fn poeorb_descriptor_binds_to_the_existing_precise_orbit_resolver() {
    let registry = registry();
    let resource = registry
        .resource("orbit.sentinel-1.poeorb.v1")
        .expect("POEORB resource");
    let ResourceDescriptor::Orbit { common, orbit } = resource else {
        panic!("POEORB must use the orbit descriptor branch");
    };
    assert_eq!(
        common.implementation_binding_id.as_deref(),
        Some("orbit-download.asf-sentinel-1.v1")
    );
    assert_eq!(orbit.orbit_product_types, ["POEORB"]);
    assert_eq!(orbit.publication_preference, ["POEORB"]);
    assert_eq!(
        OrbitResolverEndpoint::production().as_str(),
        "https://s1-orbits.asf.alaska.edu/scene/"
    );
}
