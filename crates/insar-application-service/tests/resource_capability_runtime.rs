use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, CapabilityRuntimeObservations,
    EffectiveCapabilityService, ResourceDescriptorKind, ServiceStatus,
    NISAR_REMOTE_SUBSET_CAPABILITY_ID,
};
use insar_core::{
    BuildCapabilityState, CapabilityPolicyState, CompanionRequirement, CredentialReadinessState,
    EffectiveCapabilityState, EvidenceAdmissionState, RuntimeReadinessState,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const NISAR_SUBSET_BINDING: &str = "production-worker.nisar-remote-subset.v1";
const NISAR_SUBSET_ROUTE: &str = "route.asf.nisar-rslc.remote-subset.v1";
const NISAR_SUBSET_ADMISSION: &str = "admission.nisar.remote-subset.p0.3a.v1";
const SENTINEL2_SEARCH: &str = "capability.sentinel-2.l2a.search.v1";
const SENTINEL2_MPC_SEARCH: &str = "capability.sentinel-2.l2a.mpc-search.v1";
const SENTINEL2_BINDING: &str = "cdse.sentinel-2.l2a.collection.v1";
const SENTINEL2_ROUTE: &str = "route.cdse.sentinel-2.l2a.v1";
const SENTINEL2_FULL_ARCHIVE: &str = "capability.sentinel-2.l2a.full-archive.v1";
const SENTINEL2_FULL_ARCHIVE_BINDING: &str = "cdse.sentinel-2.l2a.archive-resolver-policy.v1";
const SENTINEL2_FULL_ARCHIVE_ADMISSION: &str = "admission.sentinel-2.l2a.full-archive.p0.4a-s2.v1";
const COP30_AWS_DOWNLOAD: &str = "capability.dem.cop30.aws-download.v1";
const COP30_AWS_BINDING: &str = "dem-provider.copernicus-aws.v1";
const FABDEM_PUBLIC_DOWNLOAD: &str = "capability.dem.fabdem.download.v1";
const FABDEM_PUBLIC_BINDING: &str = "dem-provider.fabdem-public.v1";
const GEM15_DTM_PUBLIC_DOWNLOAD: &str = "capability.dem.gem15-dtm.download.v1";
const GEM15_DSM_PUBLIC_DOWNLOAD: &str = "capability.dem.gem15-dsm.download.v1";
const GEM15_PUBLIC_BINDING: &str = "dem-provider.gem15-public.v1";
const POEORB_RESOLVE: &str = "capability.sentinel-1.poeorb.resolve.v1";
const POEORB_BINDING: &str = "orbit-download.asf-sentinel-1.v1";
const SENTINEL1_STANDARD_SEARCH: &str = "capability.sentinel-1.standard.search.v1";
const SENTINEL1_STANDARD_BINDING: &str = "asf.sentinel-1.standard.v1";
const SENTINEL1_STANDARD_ROUTE: &str = "route.asf.sentinel-1.standard.v1";
const SENTINEL1_BURST_SEARCH: &str = "capability.sentinel-1.slc-burst.search.v1";
const SENTINEL1_BURST_BINDING: &str = "asf.sentinel-1.burst.v1";
const SENTINEL1_BURST_ROUTE: &str = "route.asf.sentinel-1.slc-burst.v1";

fn all_nisar_observations() -> CapabilityRuntimeObservations {
    CapabilityRuntimeObservations::default()
        .with_build(NISAR_SUBSET_BINDING, BuildCapabilityState::Available)
        .with_runtime(NISAR_SUBSET_BINDING, RuntimeReadinessState::Ready)
        .with_credential(NISAR_SUBSET_ROUTE, CredentialReadinessState::Ready)
        .with_admission(NISAR_SUBSET_ADMISSION, EvidenceAdmissionState::Accepted)
        .with_policy(
            NISAR_REMOTE_SUBSET_CAPABILITY_ID,
            CapabilityPolicyState::Allowed,
        )
}

#[test]
fn static_queries_are_registry_owned_deterministic_and_network_free() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    assert_eq!(service.status().unwrap(), ServiceStatus::LivenessOnly);

    let resources = service.list_known_resources().unwrap();
    assert!(resources
        .windows(2)
        .all(|pair| pair[0].resource_id < pair[1].resource_id));
    assert!(resources.iter().any(|resource| {
        resource.resource_id == "science.sentinel-1.slc-burst.v1"
            && resource.descriptor_kind == ResourceDescriptorKind::ScienceDataset
    }));
    assert!(resources.iter().any(|resource| {
        resource.resource_id == "orbit.sentinel-1.poeorb.v1"
            && resource.descriptor_kind == ResourceDescriptorKind::Orbit
    }));
    assert!(resources.iter().any(|resource| {
        resource.resource_id == "dem.cop30.v1"
            && resource.descriptor_kind == ResourceDescriptorKind::Dem
    }));

    let sentinel2 = service
        .resource_descriptor("science.sentinel-2.l2a.v1")
        .unwrap()
        .expect("Sentinel-2 descriptor");
    assert_eq!(sentinel2.common().display_name, "Sentinel-2 Level-2A");
    assert_eq!(
        service
            .supported_search_capabilities("science.sentinel-2.l2a.v1")
            .unwrap()
            .iter()
            .map(|capability| capability.capability_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            SENTINEL2_SEARCH,
            SENTINEL2_MPC_SEARCH,
            "capability.sentinel-2.l2a.earth-search-search.v1",
        ]
    );
    let delivery = service
        .delivery_capabilities("science.sentinel-2.l2a.v1")
        .unwrap();
    assert_eq!(delivery.len(), 1);
    assert_eq!(
        delivery[0].delivery.delivery_profile_id,
        "delivery.sentinel-2.l2a.v1"
    );
    assert!(delivery[0].delivery.supports_resume);

    assert_eq!(service.status().unwrap(), ServiceStatus::LivenessOnly);
}

#[test]
fn nisar_remote_subset_requires_every_independent_runtime_axis() {
    let ready = all_nisar_observations();
    let cases = [
        (
            CapabilityRuntimeObservations::default()
                .with_runtime(NISAR_SUBSET_BINDING, RuntimeReadinessState::Ready)
                .with_credential(NISAR_SUBSET_ROUTE, CredentialReadinessState::Ready)
                .with_admission(NISAR_SUBSET_ADMISSION, EvidenceAdmissionState::Accepted)
                .with_policy(
                    NISAR_REMOTE_SUBSET_CAPABILITY_ID,
                    CapabilityPolicyState::Allowed,
                ),
            EffectiveCapabilityState::BlockedBuild,
        ),
        (
            ready.clone().without_runtime(NISAR_SUBSET_BINDING),
            EffectiveCapabilityState::BlockedRuntime,
        ),
        (
            ready.clone().without_credential(NISAR_SUBSET_ROUTE),
            EffectiveCapabilityState::BlockedCredential,
        ),
        (
            ready.clone().without_admission(NISAR_SUBSET_ADMISSION),
            EffectiveCapabilityState::BlockedAdmission,
        ),
        (
            ready.clone().with_policy(
                NISAR_REMOTE_SUBSET_CAPABILITY_ID,
                CapabilityPolicyState::Denied,
            ),
            EffectiveCapabilityState::BlockedPolicy,
        ),
        (ready, EffectiveCapabilityState::Executable),
    ];

    for (observations, expected) in cases {
        let status = EffectiveCapabilityService::embedded(observations)
            .unwrap()
            .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
            .expect("NISAR subset capability");
        assert_eq!(status.projection.state, expected);
    }
}

#[test]
fn optical_dem_orbit_and_reserved_resources_use_the_same_projection() {
    let sentinel2 = EffectiveCapabilityService::embedded(
        CapabilityRuntimeObservations::default()
            .with_build(SENTINEL2_BINDING, BuildCapabilityState::Available)
            .with_runtime(SENTINEL2_BINDING, RuntimeReadinessState::Ready)
            .with_policy(SENTINEL2_SEARCH, CapabilityPolicyState::Allowed),
    )
    .unwrap()
    .effective_capability(SENTINEL2_SEARCH)
    .unwrap();
    assert_eq!(sentinel2.inputs.credential, None);
    assert_eq!(
        sentinel2.projection.state,
        EffectiveCapabilityState::Executable
    );

    let sentinel2_with_archive_credential = EffectiveCapabilityService::embedded(
        CapabilityRuntimeObservations::default()
            .with_build(SENTINEL2_BINDING, BuildCapabilityState::Available)
            .with_runtime(SENTINEL2_BINDING, RuntimeReadinessState::Ready)
            .with_credential(SENTINEL2_ROUTE, CredentialReadinessState::Ready)
            .with_policy(SENTINEL2_SEARCH, CapabilityPolicyState::Allowed),
    )
    .unwrap()
    .effective_capability(SENTINEL2_SEARCH)
    .unwrap();
    assert_eq!(
        sentinel2_with_archive_credential.projection.state,
        EffectiveCapabilityState::Executable
    );

    let cop30 = EffectiveCapabilityService::embedded(
        CapabilityRuntimeObservations::default()
            .with_build(COP30_AWS_BINDING, BuildCapabilityState::Available)
            .with_runtime(COP30_AWS_BINDING, RuntimeReadinessState::Ready)
            .with_policy(COP30_AWS_DOWNLOAD, CapabilityPolicyState::Allowed),
    )
    .unwrap()
    .effective_capability(COP30_AWS_DOWNLOAD)
    .unwrap();
    assert_eq!(cop30.inputs.credential, None);
    assert_eq!(cop30.projection.state, EffectiveCapabilityState::Executable);

    let poeorb = EffectiveCapabilityService::embedded(
        CapabilityRuntimeObservations::default()
            .with_build(POEORB_BINDING, BuildCapabilityState::Available)
            .with_runtime(POEORB_BINDING, RuntimeReadinessState::Ready)
            .with_policy(POEORB_RESOLVE, CapabilityPolicyState::Allowed),
    )
    .unwrap()
    .effective_capability(POEORB_RESOLVE)
    .unwrap();
    assert_eq!(poeorb.inputs.credential, None);
    assert_eq!(
        poeorb.projection.state,
        EffectiveCapabilityState::Executable
    );

    let envisat = EffectiveCapabilityService::embedded(all_nisar_observations())
        .unwrap()
        .effective_capability("capability.envisat.search.v1")
        .unwrap();
    assert_eq!(envisat.projection.state, EffectiveCapabilityState::Reserved);
}

#[test]
fn sentinel2_full_archive_requires_the_accepted_s2_admission_record() {
    let ready_without_admission = CapabilityRuntimeObservations::default()
        .with_build(
            SENTINEL2_FULL_ARCHIVE_BINDING,
            BuildCapabilityState::Available,
        )
        .with_runtime(SENTINEL2_FULL_ARCHIVE_BINDING, RuntimeReadinessState::Ready)
        .with_credential(SENTINEL2_ROUTE, CredentialReadinessState::Ready)
        .with_policy(SENTINEL2_FULL_ARCHIVE, CapabilityPolicyState::Allowed);
    let blocked = EffectiveCapabilityService::embedded(ready_without_admission)
        .unwrap()
        .effective_capability(SENTINEL2_FULL_ARCHIVE)
        .unwrap();
    assert_eq!(
        blocked.projection.state,
        EffectiveCapabilityState::BlockedAdmission
    );

    let accepted = CapabilityRuntimeObservations::default()
        .with_build(
            SENTINEL2_FULL_ARCHIVE_BINDING,
            BuildCapabilityState::Available,
        )
        .with_runtime(SENTINEL2_FULL_ARCHIVE_BINDING, RuntimeReadinessState::Ready)
        .with_credential(SENTINEL2_ROUTE, CredentialReadinessState::Ready)
        .with_admission(
            SENTINEL2_FULL_ARCHIVE_ADMISSION,
            EvidenceAdmissionState::Accepted,
        )
        .with_policy(SENTINEL2_FULL_ARCHIVE, CapabilityPolicyState::Allowed);
    let executable = EffectiveCapabilityService::embedded(accepted)
        .unwrap()
        .effective_capability(SENTINEL2_FULL_ARCHIVE)
        .unwrap();
    assert_eq!(
        executable.projection.state,
        EffectiveCapabilityState::Executable
    );
}

#[test]
fn sentinel1_standard_and_burst_fail_closed_until_their_routes_have_credentials() {
    let observations = CapabilityRuntimeObservations::default()
        .with_build(SENTINEL1_STANDARD_BINDING, BuildCapabilityState::Available)
        .with_runtime(SENTINEL1_STANDARD_BINDING, RuntimeReadinessState::Ready)
        .with_policy(SENTINEL1_STANDARD_SEARCH, CapabilityPolicyState::Allowed)
        .with_build(SENTINEL1_BURST_BINDING, BuildCapabilityState::Available)
        .with_runtime(SENTINEL1_BURST_BINDING, RuntimeReadinessState::Ready)
        .with_policy(SENTINEL1_BURST_SEARCH, CapabilityPolicyState::Allowed);
    let blocked = EffectiveCapabilityService::embedded(observations.clone()).unwrap();
    assert_eq!(
        blocked
            .effective_capability(SENTINEL1_STANDARD_SEARCH)
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::BlockedCredential
    );
    assert_eq!(
        blocked
            .effective_capability(SENTINEL1_BURST_SEARCH)
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::BlockedCredential
    );

    let ready = EffectiveCapabilityService::embedded(
        observations
            .with_credential(SENTINEL1_STANDARD_ROUTE, CredentialReadinessState::Ready)
            .with_credential(SENTINEL1_BURST_ROUTE, CredentialReadinessState::Ready),
    )
    .unwrap();
    assert_eq!(
        ready
            .effective_capability(SENTINEL1_STANDARD_SEARCH)
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        ready
            .effective_capability(SENTINEL1_BURST_SEARCH)
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
}

#[test]
fn dependency_queries_project_rules_without_creating_execution_state() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let dependencies = service
        .resource_dependencies(
            Some("science.sentinel-1.standard.v1"),
            Some("capability.sentinel-1.standard.full-archive.v1"),
            Some("planning.sar-preparation.v1"),
        )
        .unwrap();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(
        dependencies[0].dependency_id,
        "dependency.sentinel-1.standard.poeorb.v1"
    );
    match &dependencies[0].requirement {
        CompanionRequirement::Orbit {
            target_resource_id,
            required,
            matching_policy_id,
            ..
        } => {
            assert_eq!(target_resource_id, "orbit.sentinel-1.poeorb.v1");
            assert!(required);
            assert_eq!(
                matching_policy_id,
                "matching.sentinel-1.orbit.validity-then-newest.v1"
            );
        }
        other => panic!("expected orbit dependency, got {other:?}"),
    }

    let dem = service
        .resource_dependencies(None, None, Some("planning.sar-preparation.v1"))
        .unwrap();
    assert!(dem.iter().any(|dependency| {
        matches!(
            &dependency.requirement,
            CompanionRequirement::DemCoverage {
                target_resource_ids,
                required: true,
                ..
            } if target_resource_ids.contains(&"dem.cop30.v1".to_owned())
        )
    }));
    assert_eq!(service.status().unwrap(), ServiceStatus::LivenessOnly);
}

#[test]
fn application_defaults_keep_nisar_subset_default_off() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let status = service
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap()
        .expect("NISAR subset capability");
    assert_eq!(status.inputs.policy, Some(CapabilityPolicyState::Allowed));
    assert_ne!(
        status.projection.state,
        EffectiveCapabilityState::Executable
    );
}

#[test]
fn p04_nisar_production_is_compiled_and_admitted_but_remains_default_off() {
    let service = ApplicationService::new(
        ApplicationServiceConfig::liveness_only().with_capability_runtime_observations(
            CapabilityRuntimeObservations::default()
                .with_runtime(NISAR_SUBSET_BINDING, RuntimeReadinessState::Ready)
                .with_credential(NISAR_SUBSET_ROUTE, CredentialReadinessState::Missing),
        ),
    );
    let status = service
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap()
        .expect("NISAR subset capability");

    assert_eq!(
        status.inputs.build,
        Some(BuildCapabilityState::Available),
        "the normal application build must contain the production worker"
    );
    assert_eq!(
        status.inputs.policy,
        Some(CapabilityPolicyState::Allowed),
        "the capability must be product-admitted independently of credentials"
    );
    assert_eq!(
        status.projection.state,
        EffectiveCapabilityState::BlockedCredential,
        "missing Earthdata credentials must be the truthful normal-app blocker"
    );
    assert!(
        !service
            .nisar_remote_subset_capability()
            .unwrap()
            .default_enabled
    );
}

#[test]
fn offline_smoke_profile_does_not_claim_live_nisar_verification() {
    let path = std::env::temp_dir().join(format!(
        "insar-rh1-offline-smoke-capability-{}.sqlite3",
        std::process::id()
    ));
    let service = ApplicationService::new(ApplicationServiceConfig::offline_smoke(&path));
    let capability = service
        .nisar_remote_subset_capability()
        .expect("offline smoke capability is queryable");

    assert!(capability.visible);
    assert!(!capability.enabled);
    assert!(!capability.live_verified);
    assert!(!capability.production_enabled);
    assert!(!capability.default_enabled);
    assert_eq!(
        capability.reason_code.as_deref(),
        Some("NISAR_SUBSET_NOT_ENABLED")
    );

    service.shutdown().unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[test]
fn m2e_embedded_c2_evidence_and_product_policy_admit_nisar_without_default_enablement() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let status = service
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap()
        .expect("NISAR subset capability");

    assert_eq!(
        status.inputs.admission,
        Some(EvidenceAdmissionState::Accepted)
    );
    assert_eq!(status.inputs.policy, Some(CapabilityPolicyState::Allowed));
    assert_ne!(
        status.projection.state,
        EffectiveCapabilityState::Executable
    );

    let nisar = service.nisar_remote_subset_capability().unwrap();
    assert!(nisar.live_verified);
    assert!(!nisar.production_enabled);
    assert!(!nisar.enabled);
    assert!(!nisar.default_enabled);
}

#[test]
fn p04a_embedded_evidence_admits_alos2_complete_product() {
    let service = ApplicationService::new(
        ApplicationServiceConfig::liveness_only().with_capability_runtime_observations(
            CapabilityRuntimeObservations::default()
                .with_runtime(
                    "production-worker.complete.v1",
                    RuntimeReadinessState::Ready,
                )
                .with_credential(
                    "route.asf.alos2-scansar-l1.1.v1",
                    CredentialReadinessState::Ready,
                ),
        ),
    );
    let status = service
        .effective_capability("capability.alos2-scansar-l1.1.full-archive.v1")
        .unwrap()
        .expect("ALOS-2 complete-product capability");

    assert_eq!(
        status.inputs.admission,
        Some(EvidenceAdmissionState::Accepted)
    );
    assert_eq!(status.inputs.policy, Some(CapabilityPolicyState::Allowed));
    assert_eq!(
        status.projection.state,
        EffectiveCapabilityState::Executable
    );
}

#[test]
fn m2e_ordinary_config_cannot_forge_admission_or_product_policy() {
    let service = ApplicationService::new(
        ApplicationServiceConfig::liveness_only()
            .with_capability_runtime_observations(all_nisar_observations()),
    );
    let status = service
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap()
        .expect("NISAR subset capability");

    assert_eq!(
        status.inputs.admission,
        Some(EvidenceAdmissionState::Accepted)
    );
    assert_eq!(status.inputs.policy, Some(CapabilityPolicyState::Allowed));
    assert_eq!(
        status.projection.state,
        EffectiveCapabilityState::Executable
    );
}

#[cfg(feature = "nisar-subset-live-acceptance")]
#[test]
fn m2e_controlled_acceptance_seam_can_inject_a_complete_test_projection() {
    let service = ApplicationService::new(
        ApplicationServiceConfig::liveness_only()
            .with_controlled_capability_runtime_observations(all_nisar_observations()),
    );
    let status = service
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap()
        .expect("NISAR subset capability");
    assert_eq!(
        status.projection.state,
        EffectiveCapabilityState::Executable
    );
    assert!(service.nisar_remote_subset_capability().unwrap().enabled);
}

#[test]
fn effective_capability_status_is_ui_serializable_with_an_explicit_reason() {
    let status = EffectiveCapabilityService::embedded(CapabilityRuntimeObservations::default())
        .unwrap()
        .effective_capability(NISAR_REMOTE_SUBSET_CAPABILITY_ID)
        .unwrap();
    let value = serde_json::to_value(status).unwrap();
    assert_eq!(value["projection"]["state"], "blocked_build");
    assert_eq!(
        value["projection"]["reason"],
        "build_unavailable_or_unknown"
    );
    assert!(
        value["capability"]["runtime_requirements"]["admission_evidence"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn application_runtime_combines_compiled_runtime_credential_and_external_dem_owners() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("rr-g2-runtime-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let store = root.join("tasks.sqlite3");
    let config = ApplicationServiceConfig::business_store(&store)
        .with_capability_runtime_observations(
            CapabilityRuntimeObservations::default()
                .with_credential(SENTINEL2_ROUTE, CredentialReadinessState::Ready)
                .with_credential(SENTINEL1_BURST_ROUTE, CredentialReadinessState::Missing),
        );
    let service = ApplicationService::new(config);

    let before_startup = service
        .effective_capability(POEORB_RESOLVE)
        .unwrap()
        .unwrap();
    assert_eq!(
        before_startup.projection.state,
        EffectiveCapabilityState::BlockedRuntime
    );

    service.startup().unwrap();
    assert_eq!(
        service
            .effective_capability(POEORB_RESOLVE)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        service
            .effective_capability(SENTINEL1_BURST_SEARCH)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::BlockedCredential
    );
    assert_eq!(
        service
            .effective_capability(SENTINEL2_SEARCH)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );

    service.shutdown().unwrap();
    fs::remove_dir_all(root).unwrap();

    let dem_service = ApplicationService::new(
        ApplicationServiceConfig::liveness_only().with_capability_runtime_observations(
            CapabilityRuntimeObservations::default()
                .with_build(COP30_AWS_BINDING, BuildCapabilityState::Available)
                .with_runtime(COP30_AWS_BINDING, RuntimeReadinessState::Ready)
                .with_build(FABDEM_PUBLIC_BINDING, BuildCapabilityState::Available)
                .with_runtime(FABDEM_PUBLIC_BINDING, RuntimeReadinessState::Ready)
                .with_build(GEM15_PUBLIC_BINDING, BuildCapabilityState::Available)
                .with_runtime(GEM15_PUBLIC_BINDING, RuntimeReadinessState::Ready),
        ),
    );
    assert_eq!(
        dem_service
            .effective_capability(COP30_AWS_DOWNLOAD)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        dem_service
            .effective_capability(FABDEM_PUBLIC_DOWNLOAD)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        dem_service
            .effective_capability(GEM15_DTM_PUBLIC_DOWNLOAD)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        dem_service
            .effective_capability(GEM15_DSM_PUBLIC_DOWNLOAD)
            .unwrap()
            .unwrap()
            .projection
            .state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(dem_service.status().unwrap(), ServiceStatus::LivenessOnly);
}

#[test]
fn legacy_nisar_surface_consumes_the_effective_policy_result() {
    let denied = ApplicationService::new(
        ApplicationServiceConfig::liveness_only().with_capability_runtime_observations(
            all_nisar_observations().with_policy(
                NISAR_REMOTE_SUBSET_CAPABILITY_ID,
                CapabilityPolicyState::Denied,
            ),
        ),
    );
    let denied_capability = denied.nisar_remote_subset_capability().unwrap();
    assert!(denied_capability.enabled);
    assert!(denied_capability.live_verified);
    assert!(denied_capability.production_enabled);
    assert!(!denied_capability.default_enabled);

    let ordinary_injection = ApplicationService::new(
        ApplicationServiceConfig::liveness_only()
            .with_capability_runtime_observations(all_nisar_observations()),
    );
    let ordinary_capability = ordinary_injection.nisar_remote_subset_capability().unwrap();
    assert!(ordinary_capability.enabled);
    assert!(ordinary_capability.live_verified);
    assert!(ordinary_capability.production_enabled);
    assert!(!ordinary_capability.default_enabled);
}
