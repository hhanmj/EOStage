use super::*;

#[test]
fn configured_opentopography_key_enables_only_its_registered_routes() {
    let base = application_default_capability_observations()
        .with_build(
            "dem-provider.opentopography.v1",
            BuildCapabilityState::Available,
        )
        .with_runtime(
            "dem-provider.opentopography.v1",
            RuntimeReadinessState::Ready,
        );
    for status in ["set", "none", "unavailable", "unexpected"] {
        let observations = with_opentopography_readiness(base.clone(), status);
        let registry = ResourceRegistry::embedded_ref().unwrap();
        let mut routes = 0;
        for resource in &registry.resources {
            for route in &resource.common().provider_routes {
                if route.provider_id == "opentopography" {
                    routes += 1;
                    assert_eq!(
                        observations.credential_by_route.get(&route.route_id),
                        Some(&if status == "set" {
                            CredentialReadinessState::Ready
                        } else {
                            CredentialReadinessState::Missing
                        }),
                        "configuration status {status} must reach {}",
                        route.route_id,
                    );
                }
            }
        }
        assert_eq!(routes, 8);
        assert_eq!(observations.credential_by_route.len(), routes);
        let service = EffectiveCapabilityService::embedded(observations).unwrap();
        let effective = service
            .effective_capability("capability.dem.cop30.opentopography-download.v1")
            .unwrap();
        assert_eq!(
            effective.projection.state == insar_core::EffectiveCapabilityState::Executable,
            status == "set"
        );
    }
    // No observations are retained across configuration changes.
    assert!(base.credential_by_route.is_empty());
}
