use insar_core::{
    project_effective_capability, BuildCapabilityState, CapabilityKind, CapabilityPolicyState,
    CompanionRequirement, CredentialReadinessState, DemResourceSemantics,
    EffectiveCapabilityInputs, EffectiveCapabilityState, EvidenceAdmissionState, RegistryAssetRole,
    ResourceDescriptor, ResourceRegistry, RuntimeReadinessState, StaticCapabilityLifecycle,
};

fn registry() -> ResourceRegistry {
    ResourceRegistry::embedded().expect("embedded resource registry must be valid")
}

fn dem_semantics_json(nominal_resolution: &str) -> String {
    format!(
        r#"{{
            "runtime_dataset_id":"TEST",
            "coverage":"test",
            "nominal_resolution":{nominal_resolution},
            "resolution_unit":"meter",
            "horizontal_crs":"EPSG:4326",
            "vertical_datum":"test",
            "tile_scheme":"test",
            "nodata_semantics":"test"
        }}"#
    )
}

#[test]
fn dem_resolution_number_boundary_handles_workspace_arbitrary_precision() {
    let integer: DemResourceSemantics = serde_json::from_str(&dem_semantics_json("30")).unwrap();
    let decimal: DemResourceSemantics = serde_json::from_str(&dem_semantics_json("0.5")).unwrap();
    assert_eq!(integer.nominal_resolution, 30.0);
    assert_eq!(decimal.nominal_resolution, 0.5);
    assert!(serde_json::from_str::<DemResourceSemantics>(&dem_semantics_json("1e400")).is_err());
    assert!(serde_json::from_str::<DemResourceSemantics>(&dem_semantics_json("\"30\"")).is_err());
}

#[test]
fn embedded_registry_is_valid_and_has_all_closed_descriptor_branches() {
    let registry = registry();

    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/v3/resource-registry.schema.json"
    ))
    .expect("Resource Registry JSON Schema must itself be valid JSON");
    assert_eq!(
        schema.get("$schema").and_then(serde_json::Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema")
    );
    let science_required = schema
        .pointer("/$defs/scienceSemantics/required")
        .and_then(serde_json::Value::as_array)
        .expect("science semantics required fields");
    for field in ["acquisition_time_range", "sar_search"] {
        assert!(
            !science_required.iter().any(|value| value == field),
            "new optional v1 fields must not invalidate older v1 documents"
        );
    }
    assert!(schema.pointer("/$defs/sarSearchSemantics").is_some());
    assert!(schema.pointer("/$defs/sarSourceFilterSemantics").is_some());

    let science_count = registry
        .resources
        .iter()
        .filter(|resource| matches!(resource, ResourceDescriptor::ScienceDataset { .. }))
        .count();
    let orbit_count = registry
        .resources
        .iter()
        .filter(|resource| matches!(resource, ResourceDescriptor::Orbit { .. }))
        .count();
    let dem_count = registry
        .resources
        .iter()
        .filter(|resource| matches!(resource, ResourceDescriptor::Dem { .. }))
        .count();
    let atmospheric_count = registry
        .resources
        .iter()
        .filter(|resource| matches!(resource, ResourceDescriptor::Atmospheric { .. }))
        .count();

    // Issue #3 R2 B5 (2026-10-19): HLS L30 and S30 v2.0 added (search-only).
    // Milestone 2 (2026-11-09): 7 missing data sources added (4 science, 1 dem, 2 atmospheric).
    // Milestone 3 (2026-11-09): Satellite channels expansion (Sentinel-2 L1C and Sentinel-3 standard).
    // Milestone R2 Remediation (2026-11-10): 8 missing datasets added (7 science, 1 atmospheric).
    // Milestone R3 Registry Parity (2026-11-10): 12 GEE presets added (8 science, 2 dem, 2 atmospheric).
    // 4 open Earth & environmental science datasets added to the catalog
    //   (WHU CLCD v1.0.4, TUM EOT20, GEBCO 2026, NOAA ETOPO 2022; sar_search=null).
    // 2026-11-18: Open EO public datasets suite added (NASA Black Marble, NASA EMIT, DLR EnMAP, NASA ECOSTRESS -> +4 science).
    assert_eq!(science_count, 44);
    assert_eq!(orbit_count, 1);
    // 2026-11-01: NISAR DEM added; 2026-11-09: ASTER GDEM v3 added; 2026-11-10: GEE GLO30 & NASADEM added;
    // 2026-11-14: FABDEM V1-2 (dem.fabdem.v1) added;
    // 2026-11-15: GEE GEM-15 15m (science.gee.gem-15.v1) and public GCS GEM-15 DTM/DSM
    //   (dem.gem15-dtm.v1, dem.gem15-dsm.v1) added.
    assert_eq!(dem_count, 17);
    assert_eq!(atmospheric_count, 6);
}

#[test]
fn required_foundation_resources_are_migrated() {
    let registry = registry();
    for resource_id in [
        "science.nisar.l.rslc.provisional.v1",
        "science.nisar.s.ssar.rslc.v1",
        "science.alos-2.palsar-2.scansar-l1.1.v1",
        "science.alos.palsar.v1",
        "science.ers-1-2.sar.v1",
        "science.jers-1.sar.v1",
        "science.radarsat-1.sar.v1",
        "science.seasat.sar.v1",
        "science.sentinel-1.standard.v1",
        "science.sentinel-1.slc-burst.v1",
        "science.sentinel-2.l2a.v1",
        "science.landsat.c2-l2.v1",
        "science.envisat.reserved.v1",
        "orbit.sentinel-1.poeorb.v1",
        "dem.cop30.v1",
        "dem.cop90.v1",
        "dem.nasadem.v1",
        "dem.srtm-gl3.v1",
        "dem.srtm-gl1.v1",
        "dem.aw3d30.v1",
        "dem.srtm-gl1-ellipsoidal.v1",
        "dem.aw3d30-ellipsoidal.v1",
        "dem.nisar-dem.v1",
        "dem.fabdem.v1",
        "dem.gem15-dtm.v1",
        "dem.gem15-dsm.v1",
        "dem.user-local.v1",
        "science.gee.gem-15.v1",
        "atmospheric.gacos.ztd.v1",
    ] {
        assert!(
            registry.resource(resource_id).is_some(),
            "missing required resource {resource_id}"
        );
    }
}

#[test]
fn every_legacy_asf_selector_is_projected_from_one_registry_resource() {
    let registry = registry();
    // ERS, JERS-1 and RADARSAT-1 were removed from ASF's searchable
    // interfaces (Owner evidence 2026-10-31) and are retired: they keep their
    // registry identity and SAR semantics but no longer project as active
    // legacy ASF sources.
    let expected = [
        ("SENTINEL-1", "science.sentinel-1.standard.v1"),
        ("SLC-BURST", "science.sentinel-1.slc-burst.v1"),
        ("ALOS PALSAR", "science.alos.palsar.v1"),
        ("SEASAT", "science.seasat.sar.v1"),
    ];
    let projected = registry.sar_dataset_capabilities("asf.catalog");
    assert_eq!(projected.len(), expected.len());

    for (selector, resource_id) in expected {
        let resource = registry
            .resource_for_provider_selector("asf.catalog", selector)
            .expect("legacy ASF selector must resolve through ResourceRegistry");
        assert_eq!(resource.common().resource_id, resource_id);
        assert!(projected.iter().any(|dataset| dataset.dataset == selector));
    }
}

#[test]
fn asf_removed_legacy_missions_are_retired_and_not_projected_as_sources() {
    let registry = registry();
    for (resource_id, display_name) in [
        ("science.ers-1-2.sar.v1", "ERS-1 / ERS-2（ASF 已下线）"),
        ("science.jers-1.sar.v1", "JERS-1（ASF 已下线）"),
        ("science.radarsat-1.sar.v1", "RADARSAT-1（ASF 已下线）"),
    ] {
        let resource = registry.resource(resource_id).expect("retired resource");
        assert_eq!(resource.common().display_name, display_name);
        assert_eq!(
            resource.common().lifecycle,
            StaticCapabilityLifecycle::Retired
        );
        // The route identity stays declared (retired) for a future restore,
        // but neither it nor any capability is executable.
        assert!(resource.common().provider_routes.iter().all(|route| {
            route.lifecycle == StaticCapabilityLifecycle::Retired
                && !route.implementation_binding_id.is_empty()
        }));
        assert!(resource.common().capabilities.iter().all(|capability| {
            matches!(capability.lifecycle, StaticCapabilityLifecycle::Retired)
                && capability.provider_route_ids.is_empty()
        }));
        // SAR semantics stay declared so the projection can be restored if
        // ASF re-exposes the collections.
        let ResourceDescriptor::ScienceDataset { science, .. } = resource else {
            panic!("retired legacy mission keeps its science descriptor");
        };
        assert!(science.sar_search.is_some());
    }

    let sources = registry.sar_source_capabilities();
    assert_eq!(
        sources
            .iter()
            .map(|source| source.selector.as_str())
            .collect::<Vec<_>>(),
        [
            "SENTINEL-1",
            "SLC-BURST",
            "ALOS PALSAR",
            "SEASAT",
            "ALOS-2",
            "NISAR",
            "NISAR_SSAR",
            "OPERA-S1",
            "LICSAR",
        ]
    );
    assert!(
        !registry
            .resource_for_provider_selector("asf.catalog", "ERS")
            .is_some_and(
                |resource| resource.common().lifecycle == StaticCapabilityLifecycle::Implemented
            ),
        "retired ERS no longer resolves as an implemented selector"
    );
}

#[test]
fn complete_sar_source_projection_matches_typed_and_legacy_authority() {
    let registry = registry();
    let sources = registry.sar_source_capabilities();
    assert_eq!(sources.len(), 9);
    assert_eq!(
        sources
            .iter()
            .map(|source| source.selector.as_str())
            .collect::<Vec<_>>(),
        [
            "SENTINEL-1",
            "SLC-BURST",
            "ALOS PALSAR",
            "SEASAT",
            "ALOS-2",
            "NISAR",
            "NISAR_SSAR",
            "OPERA-S1",
            "LICSAR",
        ]
    );
    for selector in ["ALOS PALSAR", "SEASAT"] {
        let source = sources
            .iter()
            .find(|source| source.selector == selector)
            .expect("historical source projection");
        let capability = source
            .filters
            .get("dataset_capabilities")
            .expect("legacy source owns its complete DatasetCapabilities projection");
        assert_eq!(capability["dataset"], selector);
        assert!(capability["platforms"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()));
        let processing_levels = capability["processing_levels"]
            .as_array()
            .expect("processing level projection");
        if selector == "SEASAT" {
            assert!(processing_levels.is_empty());
            assert_eq!(capability["product_families"], serde_json::json!(["OTHER"]));
        } else {
            assert!(!processing_levels.is_empty());
        }
    }

    let alos2 = registry
        .provider_selector_binding("asf.nasa.earthdata", "ALOS-2")
        .expect("typed ALOS-2 selector binding");
    assert_eq!(
        alos2.route.implementation_binding_id,
        "asf.alos-2.palsar-2.scansar.l1.1.v1"
    );
    assert_eq!(alos2.search.beam_modes, ["WBS", "WBD"]);
    assert!(alos2.search.supports_path);
    assert!(alos2.search.supports_frame);

    let nisar = registry
        .provider_selector_binding("asf.nasa.earthdata", "NISAR")
        .expect("typed NISAR selector binding");
    let filters = nisar
        .search
        .source_filters
        .as_ref()
        .expect("NISAR typed source filters");
    assert_eq!(
        filters.options["frequency_a_polarization"],
        ["HH", "HH+HV", "VV+VH", "HH+HV+VH+VV"]
    );
    assert_eq!(
        filters.options["frequency_b_polarization"],
        ["HH", "HH+HV", "VV", "VV+VH"]
    );
    assert_eq!(
        filters.options["range_bandwidth"],
        ["5", "20+5", "40+5", "77"]
    );

    let nisar_s = registry
        .provider_selector_binding("isro.nrsc.bhoonidhi", "NISAR_SSAR")
        .expect("typed NISAR S-band selector binding");
    assert_eq!(
        nisar_s.route.native_collection_id.as_deref(),
        Some("NISAR_SSAR_RSLC")
    );
    assert_eq!(
        nisar_s.search.logical_dataset.as_deref(),
        Some("sar.nisar.s.ssar.rslc.v1")
    );
    let s_filters = nisar_s
        .search
        .source_filters
        .as_ref()
        .expect("NISAR S-band typed source filters");
    assert_eq!(
        s_filters.fixed["band"],
        insar_core::RegistryScalarValue::String("S".to_owned())
    );
    assert_eq!(
        s_filters.fixed["online"],
        insar_core::RegistryScalarValue::String("Y".to_owned())
    );
    assert_eq!(s_filters.options["mode"], ["W"]);
    assert_eq!(
        s_filters.options["polarization"],
        ["SH", "SV", "DH", "DV", "CP"]
    );
    assert_eq!(s_filters.options["acquisition_mode"], ["J", "W"]);
}

#[test]
fn sar_selector_projection_is_route_and_lifecycle_fail_closed() {
    let authority = registry();
    assert!(authority
        .provider_selector_binding("another.provider", "SENTINEL-1")
        .is_none());
    assert!(authority
        .provider_selector_binding("asf.catalog", "ENVISAT")
        .is_none());

    let mut resource_blocked = authority.clone();
    let sentinel = resource_blocked
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == "science.sentinel-1.standard.v1")
        .expect("Sentinel-1 resource");
    let ResourceDescriptor::ScienceDataset { common, .. } = sentinel else {
        panic!("Sentinel-1 must be a science resource");
    };
    common.lifecycle = StaticCapabilityLifecycle::AdmissionControlled;
    assert!(resource_blocked
        .provider_selector_binding("asf.catalog", "SENTINEL-1")
        .is_none());

    let mut route_blocked = authority.clone();
    let sentinel = route_blocked
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == "science.sentinel-1.standard.v1")
        .expect("Sentinel-1 resource");
    let ResourceDescriptor::ScienceDataset { common, .. } = sentinel else {
        panic!("Sentinel-1 must be a science resource");
    };
    common.provider_routes[0].lifecycle = StaticCapabilityLifecycle::AdmissionControlled;
    assert!(route_blocked
        .provider_selector_binding("asf.catalog", "SENTINEL-1")
        .is_none());

    let mut capability_blocked = authority;
    let sentinel = capability_blocked
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == "science.sentinel-1.standard.v1")
        .expect("Sentinel-1 resource");
    let ResourceDescriptor::ScienceDataset { common, .. } = sentinel else {
        panic!("Sentinel-1 must be a science resource");
    };
    common
        .capabilities
        .iter_mut()
        .find(|capability| capability.kind == insar_core::CapabilityKind::Search)
        .expect("Sentinel-1 search capability")
        .lifecycle = StaticCapabilityLifecycle::AdmissionControlled;
    assert!(capability_blocked
        .provider_selector_binding("asf.catalog", "SENTINEL-1")
        .is_none());
}

#[test]
fn rust_validation_rejects_noncanonical_ranges_and_invalid_search_shapes() {
    let embedded: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/v3/resource-registry.v1.json"
    ))
    .expect("registry JSON");

    let resource_index = |id: &str| {
        embedded["resources"]
            .as_array()
            .expect("resource array")
            .iter()
            .position(|resource| resource["common"]["resource_id"] == id)
            .expect("resource identity")
    };
    let sentinel_index = resource_index("science.sentinel-1.standard.v1");
    let nisar_index = resource_index("science.nisar.l.rslc.provisional.v1");

    let mut noncanonical = embedded.clone();
    *noncanonical
        .pointer_mut(&format!(
            "/resources/{sentinel_index}/science/acquisition_time_range/start"
        ))
        .expect("Sentinel archive start") =
        serde_json::Value::String("2014-10-01T08:00:00+08:00".to_owned());
    let noncanonical: ResourceRegistry =
        serde_json::from_value(noncanonical).expect("shape remains deserializable");
    assert!(noncanonical
        .validate()
        .expect_err("noncanonical timestamp must fail")
        .contains("invalid acquisition time range"));

    let mut unsupported_filter = embedded.clone();
    *unsupported_filter
        .pointer_mut(&format!(
            "/resources/{sentinel_index}/science/sar_search/frame_filter/supported"
        ))
        .expect("Sentinel frame filter") = serde_json::Value::Bool(false);
    let unsupported_filter: ResourceRegistry =
        serde_json::from_value(unsupported_filter).expect("shape remains deserializable");
    assert!(unsupported_filter
        .validate()
        .expect_err("disabled embedded filter must fail")
        .contains("invalid SAR filter descriptor"));

    let mut missing_typed_filters = embedded;
    *missing_typed_filters
        .pointer_mut(&format!(
            "/resources/{nisar_index}/science/sar_search/source_filters"
        ))
        .expect("NISAR source filters") = serde_json::Value::Null;
    let missing_typed_filters: ResourceRegistry =
        serde_json::from_value(missing_typed_filters).expect("shape remains deserializable");
    assert!(missing_typed_filters
        .validate()
        .expect_err("typed source filters must fail closed")
        .contains("no typed SAR source filters"));
}

#[test]
fn alos_palsar_and_alos2_are_distinct_science_resources() {
    let registry = registry();
    let alos1 = registry
        .resource("science.alos.palsar.v1")
        .expect("ALOS PALSAR resource");
    let alos2 = registry
        .resource("science.alos-2.palsar-2.scansar-l1.1.v1")
        .expect("ALOS-2 PALSAR-2 resource");
    assert_ne!(alos1.common().resource_id, alos2.common().resource_id);
    assert_ne!(
        alos1.common().implementation_binding_id,
        alos2.common().implementation_binding_id
    );
    assert_eq!(
        alos1.common().provider_routes[0]
            .native_collection_id
            .as_deref(),
        Some("ALOS PALSAR")
    );
    assert_eq!(
        alos2.common().provider_routes[0]
            .native_collection_id
            .as_deref(),
        Some("C3315903479-ASF")
    );
}

#[test]
fn nisar_remote_subset_is_static_admission_controlled_semantics() {
    let registry = registry();
    let capability = registry
        .capability("capability.nisar-rslc.remote-subset.v1")
        .expect("NISAR Remote Subset capability");

    assert_eq!(
        capability.lifecycle,
        StaticCapabilityLifecycle::AdmissionControlled
    );
    assert_eq!(
        capability.admission_policy_id.as_deref(),
        Some("admission.nisar.remote-subset.p0.3a.v1")
    );
    assert!(capability.runtime_requirements.build_capability);
    assert!(capability.runtime_requirements.runtime_readiness);
    assert!(capability.runtime_requirements.credential_readiness);
    assert!(capability.runtime_requirements.admission_evidence);
    assert!(capability.runtime_requirements.policy_permission);
}

#[test]
fn alos2_full_archive_is_static_admission_controlled() {
    let registry = ResourceRegistry::embedded().unwrap();
    let resource = registry
        .resource("science.alos-2.palsar-2.scansar-l1.1.v1")
        .expect("ALOS-2 resource");
    let capability = resource
        .common()
        .capabilities
        .iter()
        .find(|capability| {
            capability.capability_id == "capability.alos2-scansar-l1.1.full-archive.v1"
        })
        .expect("ALOS-2 complete-product capability");

    assert_eq!(
        capability.admission_policy_id.as_deref(),
        Some("admission.alos2-scansar-l1.1.full-archive.p0.4a.v1")
    );
    assert!(capability.runtime_requirements.admission_evidence);
}

#[test]
fn sentinel2_l2a_full_archive_is_static_admission_controlled() {
    let registry = ResourceRegistry::embedded().unwrap();
    let resource = registry
        .resource("science.sentinel-2.l2a.v1")
        .expect("Sentinel-2 resource");
    let capability = resource
        .common()
        .capabilities
        .iter()
        .find(|capability| capability.capability_id == "capability.sentinel-2.l2a.full-archive.v1")
        .expect("Sentinel-2 L2A complete-archive capability");

    assert_eq!(
        capability.admission_policy_id.as_deref(),
        Some("admission.sentinel-2.l2a.full-archive.p0.4a-s2.v1")
    );
    assert!(capability.runtime_requirements.admission_evidence);
}

#[test]
fn sentinel2_search_has_cdse_then_microsoft_but_archive_stays_cdse_only() {
    let registry = registry();
    let resource_id = "science.sentinel-2.l2a.v1";
    let search = registry
        .provider_route_candidates(resource_id, CapabilityKind::Search)
        .expect("Sentinel-2 search candidates come from the Resource Registry");
    assert_eq!(search.len(), 3);
    assert_eq!(
        search
            .iter()
            .map(|candidate| (
                candidate.route.provider_id.as_str(),
                candidate.route.priority,
                candidate.capability.capability_id.as_str(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "copernicus.dataspace.odata",
                1,
                "capability.sentinel-2.l2a.search.v1",
            ),
            (
                "microsoft.planetary-computer",
                2,
                "capability.sentinel-2.l2a.mpc-search.v1",
            ),
            (
                "element84.earth-search",
                3,
                "capability.sentinel-2.l2a.earth-search-search.v1",
            ),
        ]
    );
    let archives = registry
        .provider_route_candidates(resource_id, CapabilityKind::FullArchive)
        .expect("Sentinel-2 archive candidates remain explicit");
    assert_eq!(archives.len(), 1);
    assert_eq!(archives[0].route.provider_id, "copernicus.dataspace.odata");
    assert_ne!(search[0].route.route_id, archives[0].route.route_id);
    let missing_login = EffectiveCapabilityInputs {
        build: Some(BuildCapabilityState::Available),
        runtime: Some(RuntimeReadinessState::Ready),
        credential: Some(CredentialReadinessState::Missing),
        admission: Some(EvidenceAdmissionState::Accepted),
        policy: Some(CapabilityPolicyState::Allowed),
    };
    assert_eq!(
        project_effective_capability(search[0].capability, missing_login).state,
        EffectiveCapabilityState::Executable
    );
    assert_eq!(
        project_effective_capability(archives[0].capability, missing_login).state,
        EffectiveCapabilityState::BlockedCredential
    );
}

#[test]
fn effective_projection_is_fail_closed_and_does_not_infer_runtime_truth() {
    let registry = registry();
    let capability = registry
        .capability("capability.nisar-rslc.remote-subset.v1")
        .expect("NISAR Remote Subset capability");

    let mut inputs = EffectiveCapabilityInputs::default();
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::BlockedBuild
    );
    inputs.build = Some(BuildCapabilityState::Available);
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::BlockedRuntime
    );
    inputs.runtime = Some(RuntimeReadinessState::Ready);
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::BlockedCredential
    );
    inputs.credential = Some(CredentialReadinessState::Ready);
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::BlockedAdmission
    );
    inputs.admission = Some(EvidenceAdmissionState::Accepted);
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::BlockedPolicy
    );
    inputs.policy = Some(CapabilityPolicyState::Allowed);
    assert_eq!(
        project_effective_capability(capability, inputs).state,
        EffectiveCapabilityState::Executable
    );
}

#[test]
fn effective_projection_preserves_explicit_blocking_observations() {
    let registry = registry();
    let capability = registry
        .capability("capability.nisar-rslc.remote-subset.v1")
        .expect("NISAR Remote Subset capability");
    let ready = EffectiveCapabilityInputs {
        build: Some(BuildCapabilityState::Available),
        runtime: Some(RuntimeReadinessState::Ready),
        credential: Some(CredentialReadinessState::Ready),
        admission: Some(EvidenceAdmissionState::Accepted),
        policy: Some(CapabilityPolicyState::Allowed),
    };

    for (inputs, expected) in [
        (
            EffectiveCapabilityInputs {
                build: Some(BuildCapabilityState::Unavailable),
                ..ready
            },
            EffectiveCapabilityState::BlockedBuild,
        ),
        (
            EffectiveCapabilityInputs {
                runtime: Some(RuntimeReadinessState::NotReady),
                ..ready
            },
            EffectiveCapabilityState::BlockedRuntime,
        ),
        (
            EffectiveCapabilityInputs {
                credential: Some(CredentialReadinessState::Missing),
                ..ready
            },
            EffectiveCapabilityState::BlockedCredential,
        ),
        (
            EffectiveCapabilityInputs {
                admission: Some(EvidenceAdmissionState::Rejected),
                ..ready
            },
            EffectiveCapabilityState::BlockedAdmission,
        ),
        (
            EffectiveCapabilityInputs {
                policy: Some(CapabilityPolicyState::Denied),
                ..ready
            },
            EffectiveCapabilityState::BlockedPolicy,
        ),
    ] {
        assert_eq!(
            project_effective_capability(capability, inputs).state,
            expected
        );
    }
}

#[test]
fn landsat_search_candidates_are_ranked_and_fail_over_without_user_provider_choice() {
    let registry = registry();
    let resource_id = "science.landsat.c2-l2.v1";
    let candidates = registry
        .provider_route_candidates(resource_id, CapabilityKind::Search)
        .expect("Landsat search candidates must project from the Resource Registry");

    assert_eq!(candidates.len(), 5);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.route.provider_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "microsoft.planetary-computer",
            "element84.earth-search",
            "usgs.eros.m2m",
            "usgs.eros.m2m",
            "usgs.eros.m2m"
        ]
    );
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.route.priority)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.capability.capability_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "capability.landsat.c2-l2.search.v1",
            "capability.landsat.c2-l2.earth-search-search.v1",
            "capability.landsat.c2-l2.usgs-search.v1",
            "capability.landsat.c2-l2.usgs-search.v1",
            "capability.landsat.c2-l2.usgs-search.v1"
        ]
    );

    let ready = EffectiveCapabilityInputs {
        build: Some(BuildCapabilityState::Available),
        runtime: Some(RuntimeReadinessState::Ready),
        credential: Some(CredentialReadinessState::Ready),
        admission: Some(EvidenceAdmissionState::Accepted),
        policy: Some(CapabilityPolicyState::Allowed),
    };

    let preferred = registry
        .select_provider_route_candidate(resource_id, CapabilityKind::Search, |_, _| ready)
        .expect("candidate selection must succeed")
        .expect("at least one search route is executable");
    assert_eq!(preferred.route.provider_id, "microsoft.planetary-computer");

    let fallback = registry
        .select_provider_route_candidate(resource_id, CapabilityKind::Search, |_, route| {
            if route.provider_id == "microsoft.planetary-computer" {
                EffectiveCapabilityInputs {
                    runtime: Some(RuntimeReadinessState::NotReady),
                    ..ready
                }
            } else {
                ready
            }
        })
        .expect("candidate fallback must succeed")
        .expect("the reviewed secondary route should be executable");
    assert_eq!(fallback.route.provider_id, "element84.earth-search");

    let none = registry
        .select_provider_route_candidate(resource_id, CapabilityKind::Search, |_, route| {
            if matches!(
                route.provider_id.as_str(),
                "microsoft.planetary-computer" | "element84.earth-search"
            ) {
                EffectiveCapabilityInputs {
                    runtime: Some(RuntimeReadinessState::NotReady),
                    ..ready
                }
            } else {
                EffectiveCapabilityInputs {
                    credential: Some(CredentialReadinessState::Missing),
                    admission: Some(EvidenceAdmissionState::Rejected),
                    ..ready
                }
            }
        })
        .expect("blocked candidate evaluation must succeed");
    assert!(none.is_none());

    let asset_set = registry
        .provider_route_candidates(resource_id, CapabilityKind::AssetSet)
        .expect("asset-set candidates must project independently from search");
    assert_eq!(asset_set.len(), 1);
    assert_eq!(
        asset_set[0].route.provider_id,
        "microsoft.planetary-computer"
    );

    let mut ambiguous = registry.clone();
    let landsat = ambiguous
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == resource_id)
        .expect("Landsat resource exists");
    let common = match landsat {
        ResourceDescriptor::ScienceDataset { common, .. } => common,
        _ => panic!("Landsat must remain a science dataset"),
    };
    let mut tied_route = common.provider_routes[0].clone();
    tied_route.route_id = "route.fixture.landsat.tied.v1".to_owned();
    tied_route.provider_id = "fixture.equal-priority".to_owned();
    let tied_route_id = tied_route.route_id.clone();
    common.provider_routes.push(tied_route);
    common
        .capabilities
        .iter_mut()
        .find(|capability| capability.capability_id == "capability.landsat.c2-l2.search.v1")
        .expect("primary Landsat search capability")
        .provider_route_ids
        .push(tied_route_id);
    let error = ambiguous
        .provider_route_candidates(resource_id, CapabilityKind::Search)
        .expect_err("equal-priority distinct routes must not be ordered by accident");
    assert!(error.contains("ambiguous provider route priority"));
}

#[test]
fn catalogue_product_binding_is_explicit_source_exact_and_unique() {
    let registry = registry();
    for (logical_dataset, provider, collection, resource_id) in [
        (
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            "science.sentinel-2.l2a.v1",
        ),
        (
            "optical.landsat.c2.l2",
            "microsoft.planetary-computer",
            "landsat-c2-l2",
            "science.landsat.c2-l2.v1",
        ),
        (
            "optical.landsat.c2.l2",
            "element84.earth-search",
            "landsat-c2-l2",
            "science.landsat.c2-l2.v1",
        ),
        (
            "optical.landsat.c2.l2",
            "usgs.eros.m2m",
            "landsat_ot_c2_l2",
            "science.landsat.c2-l2.v1",
        ),
    ] {
        let resource = registry
            .catalogue_search_resource_for_source(logical_dataset, provider, collection)
            .expect("catalogue source lookup is unambiguous")
            .expect("reviewed catalogue source is registered");
        assert_eq!(resource.common().resource_id, resource_id);
    }
    assert!(registry
        .catalogue_search_resource_for_source(
            "optical.sentinel-2.l2a",
            "microsoft.planetary-computer",
            "landsat-c2-l2",
        )
        .unwrap()
        .is_none());

    let mut duplicate = registry.clone();
    let sentinel2 = duplicate
        .resources
        .iter_mut()
        .find(|resource| resource.common().resource_id == "science.sentinel-2.l2a.v1")
        .expect("Sentinel-2 resource exists");
    let ResourceDescriptor::ScienceDataset { science, .. } = sentinel2 else {
        panic!("Sentinel-2 remains a science dataset");
    };
    science.catalogue_logical_dataset = Some("optical.landsat.c2.l2".to_owned());
    assert!(duplicate
        .validate()
        .unwrap_err()
        .contains("duplicate catalogue logical dataset"));
}

#[test]
fn burst_exercises_required_multi_asset_contract() {
    let registry = registry();
    let capability = registry
        .capability("capability.sentinel-1.slc-burst.asset-set.v1")
        .expect("Burst asset-set capability");
    let resource = registry
        .resource("science.sentinel-1.slc-burst.v1")
        .expect("Burst resource");
    let schema = resource
        .common()
        .asset_schemas
        .iter()
        .find(|schema| {
            Some(schema.asset_schema_id.as_str()) == capability.asset_schema_id.as_deref()
        })
        .expect("Burst asset schema");

    let primary = schema
        .roles
        .iter()
        .find(|role| role.role == RegistryAssetRole::Primary)
        .expect("Burst science TIFF role");
    let metadata = schema
        .roles
        .iter()
        .find(|role| role.role == RegistryAssetRole::Metadata)
        .expect("Burst metadata XML role");
    assert_eq!((primary.min_count, primary.max_count), (1, Some(1)));
    assert!(primary
        .media_types
        .iter()
        .any(|value| value == "image/tiff"));
    assert_eq!((metadata.min_count, metadata.max_count), (1, Some(1)));
    assert!(metadata
        .media_types
        .iter()
        .any(|value| value == "application/xml"));
}

#[test]
fn poeorb_and_dem_and_atmospheric_dependencies_remain_declarative() {
    let registry = registry();
    assert_eq!(registry.dependencies.len(), 3);

    assert!(registry.dependencies.iter().any(|dependency| matches!(
        &dependency.requirement,
        CompanionRequirement::Orbit {
            target_resource_id,
            validity_contains_acquisition: true,
            ..
        } if target_resource_id == "orbit.sentinel-1.poeorb.v1"
    )));
    assert!(registry.dependencies.iter().any(|dependency| matches!(
        &dependency.requirement,
        CompanionRequirement::DemCoverage {
            target_resource_ids,
            coverage_contains_aoi: true,
            ..
        } if target_resource_ids.len() == 9
    )));
    assert!(registry.dependencies.iter().any(|dependency| matches!(
        &dependency.requirement,
        CompanionRequirement::AtmosphericDateAoi {
            target_resource_id,
            required: false,
            match_acquisition_dates: true,
            bind_processing_aoi: true,
            ..
        } if target_resource_id == "atmospheric.gacos.ztd.v1"
    )));
}

#[test]
fn envisat_is_truthfully_reserved_even_if_runtime_inputs_claim_ready() {
    let registry = registry();
    let resource = registry
        .resource("science.envisat.reserved.v1")
        .expect("ENVISAT reserved resource");
    assert_eq!(
        resource.common().lifecycle,
        StaticCapabilityLifecycle::Reserved
    );
    assert!(resource.common().provider_routes.is_empty());

    let capability = registry
        .capability("capability.envisat.search.v1")
        .expect("ENVISAT reserved capability");
    let projection = project_effective_capability(
        capability,
        EffectiveCapabilityInputs {
            build: Some(BuildCapabilityState::Available),
            runtime: Some(RuntimeReadinessState::Ready),
            credential: Some(CredentialReadinessState::Ready),
            admission: Some(EvidenceAdmissionState::Accepted),
            policy: Some(CapabilityPolicyState::Allowed),
        },
    );
    assert_eq!(projection.state, EffectiveCapabilityState::Reserved);
}

#[test]
fn integrity_rejects_duplicate_static_authority_identity() {
    let mut registry = registry();
    registry.resources.push(registry.resources[0].clone());
    assert!(registry
        .validate()
        .expect_err("duplicate resource identity must fail")
        .contains("duplicate resource_id"));
}
