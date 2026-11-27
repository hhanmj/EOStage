use geo::{Coord, LineString, MultiPolygon, Polygon};
use insar_core::{
    canonicalize_nisar_subset_geojson_v2, canonicalize_nisar_subset_multipolygon_v2,
    derive_nisar_subset_dependency_closure_v2, derive_nisar_subset_geometry_authority_v2,
    NisarSubsetByteEstimateV2, NisarSubsetChecksumAlgorithm, NisarSubsetChecksumV1,
    NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyKindV2,
    NisarSubsetDependencyRelationV2, NisarSubsetFrequency, NisarSubsetFrequencyCapabilityV2,
    NisarSubsetGeometryInvalidReasonV2, NisarSubsetGeometryRoleV2, NisarSubsetPlanDraftV2,
    NisarSubsetPlanMetadataEvidenceV2, NisarSubsetPlanV2, NisarSubsetPlanV2Error,
    NisarSubsetPreparedIntentBuildInputV2, NisarSubsetPreparedIntentV2, NisarSubsetQualityStatusV2,
    NisarSubsetSourceAssetIdentityV2, NisarSubsetSourceCapabilityV2,
    NisarSubsetSourceDependencyEdgeV2, NisarSubsetSourceDependencyGraphV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceDependencyNodeV2,
    NisarSubsetSourceProductIdentityV2, NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use serde_json::{json, Map, Value};

fn geometry(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn rectangle(west: f64, south: f64, east: f64, north: f64) -> Map<String, Value> {
    geometry(json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    }))
}

fn product_identity() -> NisarSubsetSourceProductIdentityV2 {
    NisarSubsetSourceProductIdentityV2 {
        provider_id: "asf.nasa.earthdata".to_owned(),
        collection_id: "C2853145197-ASF".to_owned(),
        collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
        provider_product_id: "G9900000001-ASF".to_owned(),
        provider_product_key: "PROVIDER::asf::ITEM::G9900000001-ASF".to_owned(),
        product_specification_version: "1.5.0".to_owned(),
        crid: "P05023".to_owned(),
        quality_status: NisarSubsetQualityStatusV2::Provisional,
    }
}

fn asset_identity() -> NisarSubsetSourceAssetIdentityV2 {
    NisarSubsetSourceAssetIdentityV2 {
        asset_key: "science-rslc-hdf5".to_owned(),
        stable_asset_key: "PROVIDER::asf::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5"
            .to_owned(),
        file_name: "NISAR_SOURCE.h5".to_owned(),
        checksum: Some(NisarSubsetChecksumV1 {
            algorithm: NisarSubsetChecksumAlgorithm::Sha256,
            value: "a".repeat(64),
        }),
    }
}

fn source_capability() -> NisarSubsetSourceCapabilityV2 {
    NisarSubsetSourceCapabilityV2 {
        evidence_sha256: "b".repeat(64),
        frequencies: vec![
            NisarSubsetFrequencyCapabilityV2 {
                frequency: NisarSubsetFrequency::B,
                available_polarizations: vec!["VV".to_owned()],
            },
            NisarSubsetFrequencyCapabilityV2 {
                frequency: NisarSubsetFrequency::A,
                available_polarizations: vec!["HV".to_owned(), "HH".to_owned()],
            },
        ],
    }
}

fn source_node(
    kind: NisarSubsetSourceDependencyNodeKindV2,
    source_path: impl Into<String>,
) -> NisarSubsetSourceDependencyNodeV2 {
    NisarSubsetSourceDependencyNodeV2 {
        source_path: source_path.into(),
        kind,
        attribute_name: (kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
            .then(|| "Conventions".to_owned()),
    }
}

fn source_edge(
    dependent_path: impl Into<String>,
    dependency_path: impl Into<String>,
    relation: NisarSubsetDependencyRelationV2,
) -> NisarSubsetSourceDependencyEdgeV2 {
    NisarSubsetSourceDependencyEdgeV2 {
        dependent_path: dependent_path.into(),
        dependency_path: dependency_path.into(),
        relation,
    }
}

fn dependency_graph() -> NisarSubsetSourceDependencyGraphV2 {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    let identification = "/science/LSAR/identification/granuleId";
    let provenance = "/science/LSAR/identification/productVersion";
    let attribute = "/";
    let other = "/science/LSAR/RSLC/metadata/orbit";
    let mut nodes = vec![
        source_node(Kind::IdentificationMetadata, identification),
        source_node(Kind::ProvenanceMetadata, provenance),
        source_node(Kind::Attribute, attribute),
        source_node(Kind::OtherScientificDependency, other),
    ];
    let mut edges = Vec::new();
    for (frequency, polarizations) in [("A", &["HH", "HV"][..]), ("B", &["VV"][..])] {
        let coordinate =
            format!("/science/LSAR/RSLC/metadata/geolocationGrid/frequency{frequency}/coordinateX");
        let dimension = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/zeroDopplerTime");
        let mask = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/inputDataExceptionMask");
        nodes.extend([
            source_node(Kind::CoordinateDataset, &coordinate),
            source_node(Kind::DimensionScale, &dimension),
            source_node(Kind::ValidityMask, &mask),
        ]);
        for polarization in polarizations {
            let science = format!("/science/LSAR/RSLC/swaths/frequency{frequency}/{polarization}");
            nodes.push(source_node(Kind::ScienceDataset, &science));
            edges.extend([
                source_edge(&science, &coordinate, Relation::Coordinate),
                source_edge(&science, &dimension, Relation::DimensionScale),
                source_edge(&science, &mask, Relation::ValidityMask),
                source_edge(&science, identification, Relation::RequiredMetadata),
                source_edge(&science, provenance, Relation::Provenance),
                source_edge(&science, attribute, Relation::RequiredAttribute),
                source_edge(&science, other, Relation::OtherScientificDependency),
            ]);
        }
    }
    nodes.reverse();
    edges.reverse();
    NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2::ReviewedSourceEvidenceV1,
        evidence_sha256: "c".repeat(64),
        nodes,
        edges,
    }
}

fn draft(
    source_footprint: Map<String, Value>,
    requested_geometry: Map<String, Value>,
) -> NisarSubsetPlanDraftV2 {
    NisarSubsetPlanDraftV2 {
        source_product_identity: product_identity(),
        source_asset_identity: asset_identity(),
        source_capability: source_capability(),
        source_footprint,
        requested_geometry,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HV".to_owned(), "HH".to_owned(), "HH".to_owned()],
        source_dependency_graph: dependency_graph(),
        source_estimate: Some(NisarSubsetByteEstimateV2 {
            bytes: 14_252_244_992,
            evidence_sha256: "d".repeat(64),
        }),
        transfer_estimate: None,
        derived_estimate: None,
        warnings: vec!["PROVISIONAL source product".to_owned()],
    }
}

fn plan() -> NisarSubsetPlanV2 {
    NisarSubsetPlanV2::build(draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    ))
    .unwrap()
}

fn prepared_intent(
    source_footprint: Map<String, Value>,
    requested_geometry: Map<String, Value>,
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: product_identity(),
        source_asset_identity: asset_identity(),
        source_capability: source_capability(),
        source_footprint,
        requested_geometry,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HV".to_owned(), "HH".to_owned(), "HH".to_owned()],
    })
    .unwrap()
}

fn metadata_evidence(warnings: Vec<String>) -> NisarSubsetPlanMetadataEvidenceV2 {
    NisarSubsetPlanMetadataEvidenceV2 {
        source_estimate: Some(NisarSubsetByteEstimateV2 {
            bytes: 14_252_244_992,
            evidence_sha256: "d".repeat(64),
        }),
        transfer_estimate: None,
        derived_estimate: None,
        warnings,
    }
}

fn sband_product_identity() -> NisarSubsetSourceProductIdentityV2 {
    NisarSubsetSourceProductIdentityV2 {
        provider_id: "isro.nrsc.bhoonidhi".to_owned(),
        collection_id: "NISAR_SSAR_RSLC".to_owned(),
        collection_profile_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1".to_owned(),
        provider_product_id: "NISAR_S1_PR_RSLC_FIXTURE".to_owned(),
        provider_product_key: "PROVIDER::bhoonidhi::ITEM::NISAR_S1_PR_RSLC_FIXTURE".to_owned(),
        product_specification_version: "1.2.1".to_owned(),
        crid: "P00500".to_owned(),
        quality_status: NisarSubsetQualityStatusV2::Provisional,
    }
}

fn sband_source_capability() -> NisarSubsetSourceCapabilityV2 {
    NisarSubsetSourceCapabilityV2 {
        evidence_sha256: "e".repeat(64),
        frequencies: vec![NisarSubsetFrequencyCapabilityV2 {
            frequency: NisarSubsetFrequency::A,
            available_polarizations: vec!["RH".to_owned(), "RV".to_owned()],
        }],
    }
}

fn sband_dependency_graph() -> NisarSubsetSourceDependencyGraphV2 {
    let mut graph = dependency_graph();
    let translate = |value: &str| {
        value
            .replace("/science/LSAR/", "/science/SSAR/")
            .replace("/frequencyA/HH", "/frequencyA/RH")
            .replace("/frequencyA/HV", "/frequencyA/RV")
    };
    for node in &mut graph.nodes {
        node.source_path = translate(&node.source_path);
    }
    for edge in &mut graph.edges {
        edge.dependent_path = translate(&edge.dependent_path);
        edge.dependency_path = translate(&edge.dependency_path);
    }
    graph
}

#[test]
fn sband_plan_binds_dependency_closure_to_ssar_source_identity() {
    let mut sband = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    sband.source_product_identity = sband_product_identity();
    sband.source_capability = sband_source_capability();
    sband.source_dependency_graph = sband_dependency_graph();
    sband.selected_frequency = NisarSubsetFrequency::A;
    sband.selected_polarizations = vec!["RV".to_owned(), "RH".to_owned()];

    let plan = NisarSubsetPlanV2::build(sband).expect("reviewed S-band plan");
    let science = plan
        .required_dependencies
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
        .map(|item| item.source_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        science,
        [
            "/science/SSAR/RSLC/swaths/frequencyA/RH",
            "/science/SSAR/RSLC/swaths/frequencyA/RV",
        ]
    );
    assert!(plan.required_dependencies.items.iter().any(|item| {
        item.kind == NisarSubsetDependencyKindV2::ValidityMask
            && item.source_path == "/science/SSAR/RSLC/swaths/frequencyA/inputDataExceptionMask"
    }));
    assert!(plan
        .required_dependencies
        .items
        .iter()
        .all(|item| !item.source_path.starts_with("/science/LSAR/")));

    let mut mismatched = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    mismatched.source_product_identity = sband_product_identity();
    mismatched.source_capability = sband_source_capability();
    mismatched.selected_frequency = NisarSubsetFrequency::A;
    mismatched.selected_polarizations = vec!["RH".to_owned(), "RV".to_owned()];
    assert!(matches!(
        NisarSubsetPlanV2::build(mismatched),
        Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(_))
    ));
}

#[test]
fn normal_polygon_uses_true_effective_intersection_and_preserves_decimals() {
    let plan = plan();
    assert_eq!(plan.requested_geometry, plan.effective_geometry);
    assert_eq!(plan.selected_polarizations, ["HH", "HV"]);
    assert!(plan.canonical_json().unwrap().contains("2.123456789012345"));
    assert_eq!(
        plan.provenance.product_semantics,
        NISAR_SUBSET_PLAN_V2_PRODUCT_SEMANTICS
    );
    assert_eq!(
        plan.provenance.plan_identity_sha256,
        plan.canonical_semantic_plan_digest
    );
}

#[test]
fn polygon_hole_and_multipolygon_topology_are_preserved() {
    let with_hole = geometry(json!({
        "type": "Polygon",
        "coordinates": [
            [[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0], [1.0, 1.0]],
            [[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0], [4.0, 4.0]]
        ]
    }));
    let authority =
        derive_nisar_subset_geometry_authority_v2(&rectangle(0.0, 0.0, 10.0, 10.0), &with_hole)
            .unwrap();
    assert_eq!(authority.effective_geometry.ring_count, 2);
    assert_eq!(
        authority.effective_geometry.canonical_geojson["coordinates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let multi = geometry(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[7.0, 7.0], [8.0, 7.0], [8.0, 8.0], [7.0, 8.0], [7.0, 7.0]]],
            [[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0], [2.0, 2.0]]]
        ]
    }));
    let authority =
        derive_nisar_subset_geometry_authority_v2(&rectangle(0.0, 0.0, 10.0, 10.0), &multi)
            .unwrap();
    assert_eq!(authority.requested_geometry.component_count, 2);
    assert_eq!(authority.effective_geometry.component_count, 2);
    assert_eq!(
        authority.effective_geometry.canonical_geojson["type"],
        "MultiPolygon"
    );
}

#[test]
fn authoritative_administrative_polygon_over_two_thousand_vertices_is_admitted_for_subset() {
    let vertex_count = 2_400usize;
    let mut ring = Vec::with_capacity(vertex_count + 1);
    for index in 0..vertex_count {
        let angle = (index as f64) * std::f64::consts::TAU / (vertex_count as f64);
        let lon = 85.0 + 6.0 * angle.cos();
        let lat = 31.0 + 3.0 * angle.sin();
        ring.push(json!([lon, lat]));
    }
    ring.push(ring[0].clone());
    let large_admin_aoi = geometry(json!({
        "type": "Polygon",
        "coordinates": [ring]
    }));
    let footprint = rectangle(79.5, 29.5, 82.0, 32.0);
    let authority =
        derive_nisar_subset_geometry_authority_v2(&footprint, &large_admin_aoi).unwrap();
    assert_eq!(
        authority.requested_geometry.vertex_count,
        (vertex_count + 1) as u32
    );
    assert!(authority.effective_geometry.vertex_count >= 5);
}

#[test]
fn partial_intersection_and_requested_larger_than_footprint_use_polygon_authority() {
    let requested = rectangle(3.0, -1.0, 7.0, 3.0);
    let authority =
        derive_nisar_subset_geometry_authority_v2(&rectangle(0.0, 0.0, 5.0, 5.0), &requested)
            .unwrap();
    let expected = canonicalize_nisar_subset_geojson_v2(
        &rectangle(3.0, 0.0, 5.0, 3.0),
        NisarSubsetGeometryRoleV2::Effective,
    )
    .unwrap();
    assert_eq!(authority.effective_geometry, expected);

    let footprint = rectangle(1.0, 1.0, 4.0, 4.0);
    let authority =
        derive_nisar_subset_geometry_authority_v2(&footprint, &rectangle(0.0, 0.0, 8.0, 8.0))
            .unwrap();
    let expected =
        canonicalize_nisar_subset_geojson_v2(&footprint, NisarSubsetGeometryRoleV2::Effective)
            .unwrap();
    assert_eq!(authority.effective_geometry, expected);
}

#[test]
fn empty_self_intersecting_degenerate_and_out_of_range_geometry_fail_closed() {
    let source = rectangle(0.0, 0.0, 5.0, 5.0);
    assert_eq!(
        derive_nisar_subset_geometry_authority_v2(&source, &rectangle(6.0, 6.0, 7.0, 7.0)),
        Err(NisarSubsetPlanV2Error::EmptyEffectiveGeometry)
    );
    for invalid in [
        geometry(json!({
            "type": "Polygon",
            "coordinates": [[[1.0, 1.0], [4.0, 4.0], [1.0, 4.0], [4.0, 1.0], [1.0, 1.0]]]
        })),
        geometry(json!({
            "type": "Polygon",
            "coordinates": [[[1.0, 1.0], [2.0, 1.0], [3.0, 1.0], [1.0, 1.0]]]
        })),
    ] {
        assert!(matches!(
            derive_nisar_subset_geometry_authority_v2(&source, &invalid),
            Err(NisarSubsetPlanV2Error::GeometryInvalid {
                geometry: NisarSubsetGeometryRoleV2::Requested,
                reason: NisarSubsetGeometryInvalidReasonV2::InvalidTopology,
            })
        ));
    }
    let invalid_coordinate = rectangle(181.0, 1.0, 182.0, 2.0);
    assert!(matches!(
        derive_nisar_subset_geometry_authority_v2(&source, &invalid_coordinate),
        Err(NisarSubsetPlanV2Error::GeometryInvalid {
            reason: NisarSubsetGeometryInvalidReasonV2::CoordinateOutOfRange,
            ..
        })
    ));
}

#[test]
fn non_finite_coordinate_and_antimeridian_have_closed_typed_errors() {
    for invalid_x in [f64::NAN, f64::INFINITY] {
        let non_finite = MultiPolygon(vec![Polygon::new(
            LineString::new(vec![
                Coord { x: 0.0, y: 0.0 },
                Coord {
                    x: invalid_x,
                    y: 0.0,
                },
                Coord { x: 1.0, y: 1.0 },
                Coord { x: 0.0, y: 0.0 },
            ]),
            Vec::new(),
        )]);
        assert!(matches!(
            canonicalize_nisar_subset_multipolygon_v2(
                &non_finite,
                NisarSubsetGeometryRoleV2::Requested
            ),
            Err(NisarSubsetPlanV2Error::GeometryInvalid {
                reason: NisarSubsetGeometryInvalidReasonV2::InvalidCoordinate,
                ..
            })
        ));
    }

    let crossing = geometry(json!({
        "type": "Polygon",
        "coordinates": [[[179.0, -1.0], [-179.0, -1.0], [-179.0, 1.0], [179.0, 1.0], [179.0, -1.0]]]
    }));
    assert_eq!(
        derive_nisar_subset_geometry_authority_v2(&rectangle(-10.0, -10.0, 10.0, 10.0), &crossing),
        Err(NisarSubsetPlanV2Error::AntimeridianUnsupported {
            geometry: NisarSubsetGeometryRoleV2::Requested,
        })
    );
}

#[test]
fn json_roundtrip_and_semantic_digest_are_stable() {
    let first = plan();
    let encoded = serde_json::to_string(&first).unwrap();
    let roundtrip: NisarSubsetPlanV2 = serde_json::from_str(&encoded).unwrap();
    roundtrip.validate().unwrap();
    assert_eq!(
        first.canonical_json().unwrap(),
        roundtrip.canonical_json().unwrap()
    );
    assert_eq!(
        first.canonical_semantic_plan_digest,
        roundtrip.canonical_semantic_plan_digest
    );
    assert_eq!(
        first.source_footprint_geometry,
        roundtrip.source_footprint_geometry
    );
    assert_eq!(
        first.source_footprint_sha256,
        roundtrip.source_footprint_sha256
    );
}

#[test]
fn key_ring_component_and_dependency_order_do_not_change_identity() {
    let canonical = geometry(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0], [2.0, 2.0]]],
            [[[6.0, 6.0], [7.0, 6.0], [7.0, 7.0], [6.0, 7.0], [6.0, 6.0]]]
        ]
    }));
    let mut reordered = Map::new();
    reordered.insert(
        "coordinates".to_owned(),
        json!([
            [[[7.0, 7.0], [7.0, 6.0], [6.0, 6.0], [6.0, 7.0], [7.0, 7.0]]],
            [[[3.0, 3.0], [3.0, 2.0], [2.0, 2.0], [2.0, 3.0], [3.0, 3.0]]]
        ]),
    );
    reordered.insert("type".to_owned(), json!("MultiPolygon"));
    let first =
        NisarSubsetPlanV2::build(draft(rectangle(0.0, 0.0, 10.0, 10.0), canonical)).unwrap();
    let mut reordered_draft = draft(rectangle(0.0, 0.0, 10.0, 10.0), reordered);
    reordered_draft.source_dependency_graph.nodes.reverse();
    reordered_draft.source_dependency_graph.edges.reverse();
    let second = NisarSubsetPlanV2::build(reordered_draft).unwrap();
    assert_eq!(
        first.canonical_geometry_digest,
        second.canonical_geometry_digest
    );
    assert_eq!(
        first.canonical_semantic_plan_digest,
        second.canonical_semantic_plan_digest
    );
}

#[test]
fn polarization_duplicates_normalize_but_noncanonical_persisted_plan_is_rejected() {
    let plan = plan();
    assert_eq!(plan.selected_polarizations, ["HH", "HV"]);
    let mut duplicate = plan;
    duplicate.selected_polarizations.push("HH".to_owned());
    assert!(matches!(
        duplicate.validate(),
        Err(NisarSubsetPlanV2Error::PolarizationInvalid(_))
    ));
}

#[test]
fn circular_sband_polarizations_are_canonical_subset_values() {
    let capability = NisarSubsetSourceCapabilityV2 {
        evidence_sha256: "e".repeat(64),
        frequencies: vec![NisarSubsetFrequencyCapabilityV2 {
            frequency: NisarSubsetFrequency::A,
            available_polarizations: vec!["RV".to_owned(), "RH".to_owned()],
        }],
    };
    let intent = NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: product_identity(),
        source_asset_identity: asset_identity(),
        source_capability: capability,
        source_footprint: rectangle(0.0, 0.0, 10.0, 10.0),
        requested_geometry: rectangle(2.0, 2.0, 8.0, 8.0),
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["RV".to_owned(), "RH".to_owned()],
    })
    .expect("circular S-band polarization vocabulary is valid");

    assert_eq!(intent.selected_polarizations, ["RH", "RV"]);
    assert_eq!(
        intent.source_capability.frequencies[0].available_polarizations,
        ["RH", "RV"]
    );
}

#[test]
fn source_identity_and_effective_geometry_changes_change_semantic_identity() {
    let first = plan();
    let mut changed_source = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    );
    changed_source.source_product_identity.provider_product_id = "G9900000002-ASF".to_owned();
    let changed_source = NisarSubsetPlanV2::build(changed_source).unwrap();
    assert_ne!(
        first.canonical_semantic_plan_digest,
        changed_source.canonical_semantic_plan_digest
    );

    let changed_effective = NisarSubsetPlanV2::build(draft(
        rectangle(0.0, 0.0, 5.0, 5.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    ))
    .unwrap();
    assert_ne!(
        first.canonical_geometry_digest.effective_sha256,
        changed_effective.canonical_geometry_digest.effective_sha256
    );
    assert_ne!(
        first.canonical_semantic_plan_digest,
        changed_effective.canonical_semantic_plan_digest
    );
}

#[test]
fn unknown_or_incomplete_dependencies_and_non_range_mode_fail_closed() {
    let mut incomplete = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    incomplete.source_dependency_graph.nodes[0].kind =
        NisarSubsetSourceDependencyNodeKindV2::Unknown;
    assert!(matches!(
        NisarSubsetPlanV2::build(incomplete),
        Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(_))
    ));

    let mut value = serde_json::to_value(plan()).unwrap();
    value["required_dependencies"]["items"][0]["kind"] = json!("unknown_dependency");
    assert!(serde_json::from_value::<NisarSubsetPlanV2>(value).is_err());
    let mut value = serde_json::to_value(plan()).unwrap();
    value["transfer_mode"] = json!("complete_local");
    assert!(serde_json::from_value::<NisarSubsetPlanV2>(value).is_err());
}

#[test]
fn source_footprint_is_frozen_canonical_and_changes_semantic_identity() {
    let first = plan();
    let second = NisarSubsetPlanV2::build(draft(
        rectangle(-1.0, -1.0, 11.0, 11.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    ))
    .unwrap();
    assert_eq!(first.effective_geometry, second.effective_geometry);
    assert_ne!(
        first.source_footprint_sha256,
        second.source_footprint_sha256
    );
    assert_ne!(
        first.canonical_semantic_plan_digest,
        second.canonical_semantic_plan_digest
    );
    assert_eq!(
        first.provenance.source_footprint_sha256,
        first.source_footprint_sha256
    );

    for (stored, role) in [
        (
            &first.source_footprint_geometry,
            NisarSubsetGeometryRoleV2::SourceFootprint,
        ),
        (
            &first.requested_geometry,
            NisarSubsetGeometryRoleV2::Requested,
        ),
        (
            &first.effective_geometry,
            NisarSubsetGeometryRoleV2::Effective,
        ),
    ] {
        assert_eq!(
            canonicalize_nisar_subset_geojson_v2(&stored.canonical_geojson, role).unwrap(),
            *stored
        );
    }
}

#[test]
fn geometry_authority_rejects_outside_or_non_intersection_effective_geometry() {
    let mut outside_request = plan();
    outside_request.effective_geometry = canonicalize_nisar_subset_geojson_v2(
        &rectangle(1.0, 1.0, 3.0, 3.0),
        NisarSubsetGeometryRoleV2::Effective,
    )
    .unwrap();
    assert!(matches!(
        outside_request.validate(),
        Err(NisarSubsetPlanV2Error::IdentityMismatch(_))
    ));

    let mut outside_source = NisarSubsetPlanV2::build(draft(
        rectangle(2.0, 2.0, 8.0, 8.0),
        rectangle(0.0, 0.0, 10.0, 10.0),
    ))
    .unwrap();
    outside_source.effective_geometry = canonicalize_nisar_subset_geojson_v2(
        &rectangle(1.0, 1.0, 3.0, 3.0),
        NisarSubsetGeometryRoleV2::Effective,
    )
    .unwrap();
    assert!(matches!(
        outside_source.validate(),
        Err(NisarSubsetPlanV2Error::IdentityMismatch(_))
    ));

    let mut smaller_but_legal = plan();
    smaller_but_legal.effective_geometry = canonicalize_nisar_subset_geojson_v2(
        &rectangle(3.0, 3.0, 7.0, 7.0),
        NisarSubsetGeometryRoleV2::Effective,
    )
    .unwrap();
    assert!(matches!(
        smaller_but_legal.validate(),
        Err(NisarSubsetPlanV2Error::IdentityMismatch(_))
    ));

    let mut footprint_digest_tamper = plan();
    footprint_digest_tamper.source_footprint_sha256 = "0".repeat(64);
    assert!(matches!(
        footprint_digest_tamper.validate(),
        Err(NisarSubsetPlanV2Error::IdentityMismatch(_))
    ));
}

#[test]
fn dependency_planner_derives_exact_science_paths_and_shared_dependencies() {
    let graph = dependency_graph();
    let hh = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::A,
        &["HH".to_owned()],
    )
    .unwrap();
    let hh_science = hh
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
        .map(|item| item.source_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(hh_science, ["/science/LSAR/RSLC/swaths/frequencyA/HH"]);
    assert!(hh.complete);
    assert_eq!(hh.unknown_dependency_count, 0);

    let dual = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::A,
        &["HV".to_owned(), "HH".to_owned()],
    )
    .unwrap();
    let science = dual
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
        .map(|item| item.source_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        science,
        [
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HV"
        ]
    );
    let coordinate = dual
        .items
        .iter()
        .find(|item| item.kind == NisarSubsetDependencyKindV2::CoordinateDataset)
        .unwrap();
    assert_eq!(
        coordinate.required_by,
        [
            "/science/LSAR/RSLC/swaths/frequencyA/HH",
            "/science/LSAR/RSLC/swaths/frequencyA/HV"
        ]
    );

    let frequency_b = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::B,
        &["VV".to_owned()],
    )
    .unwrap();
    assert!(frequency_b.items.iter().any(|item| {
        item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset
            && item.source_path == "/science/LSAR/RSLC/swaths/frequencyB/VV"
    }));
    assert!(frequency_b.items.iter().any(|item| {
        item.kind == NisarSubsetDependencyKindV2::ValidityMask
            && item.source_path == "/science/LSAR/RSLC/swaths/frequencyB/inputDataExceptionMask"
    }));
}

#[test]
fn standalone_dependency_closure_infers_ssar_root_and_rejects_mixed_science_roots() {
    let graph = sband_dependency_graph();
    let closure = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::A,
        &["RH".to_owned(), "RV".to_owned()],
    )
    .expect("standalone closure must infer the reviewed SSAR swaths root");
    let science = closure
        .items
        .iter()
        .filter(|item| item.kind == NisarSubsetDependencyKindV2::SelectedScienceDataset)
        .map(|item| item.source_path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        science,
        [
            "/science/SSAR/RSLC/swaths/frequencyA/RH",
            "/science/SSAR/RSLC/swaths/frequencyA/RV",
        ]
    );
    assert!(closure.items.iter().any(|item| {
        item.kind == NisarSubsetDependencyKindV2::ValidityMask
            && item.source_path == "/science/SSAR/RSLC/swaths/frequencyA/inputDataExceptionMask"
    }));

    let mut mixed = graph;
    let old = "/science/SSAR/RSLC/swaths/frequencyA/RV";
    let new = "/science/LSAR/RSLC/swaths/frequencyA/RV";
    for node in &mut mixed.nodes {
        if node.source_path == old {
            node.source_path = new.to_owned();
        }
    }
    for edge in &mut mixed.edges {
        if edge.dependent_path == old {
            edge.dependent_path = new.to_owned();
        }
        if edge.dependency_path == old {
            edge.dependency_path = new.to_owned();
        }
    }
    assert!(matches!(
        derive_nisar_subset_dependency_closure_v2(
            &mixed,
            &NisarSubsetFrequency::A,
            &["RH".to_owned(), "RV".to_owned()]
        ),
        Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(_))
    ));
}

#[test]
fn canonical_hdf5_root_is_valid_only_as_a_path_while_noncanonical_forms_remain_closed() {
    let graph_with_attribute_path = |attribute_path: &str| {
        let mut graph = dependency_graph();
        let old_path = graph
            .nodes
            .iter()
            .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
            .unwrap()
            .source_path
            .clone();
        graph
            .nodes
            .iter_mut()
            .find(|node| node.source_path == old_path)
            .unwrap()
            .source_path = attribute_path.to_owned();
        for edge in &mut graph.edges {
            if edge.dependency_path == old_path {
                edge.dependency_path = attribute_path.to_owned();
            }
        }
        graph
    };

    let root = graph_with_attribute_path("/");
    let closure = derive_nisar_subset_dependency_closure_v2(
        &root,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .unwrap();
    assert!(closure.items.iter().any(|item| {
        item.kind == NisarSubsetDependencyKindV2::Attribute && item.source_path == "/"
    }));

    let mut root_with_non_attribute_kind = root.clone();
    root_with_non_attribute_kind
        .nodes
        .iter_mut()
        .find(|node| node.source_path == "/")
        .unwrap()
        .kind = NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency;
    for edge in &mut root_with_non_attribute_kind.edges {
        if edge.dependency_path == "/" {
            edge.relation = NisarSubsetDependencyRelationV2::OtherScientificDependency;
        }
    }
    assert!(derive_nisar_subset_dependency_closure_v2(
        &root_with_non_attribute_kind,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .is_err());

    for invalid in [
        "",
        "//",
        "/science//LSAR",
        "/science/",
        "/science/./LSAR",
        "/science/LSAR/..",
        "/science/../LSAR",
        "/science\\LSAR",
    ] {
        assert!(derive_nisar_subset_dependency_closure_v2(
            &graph_with_attribute_path(invalid),
            &NisarSubsetFrequency::A,
            &["HH".to_owned(), "HV".to_owned()],
        )
        .is_err());
    }
}

#[test]
fn attribute_nodes_require_an_exact_valid_name_and_non_attributes_cannot_carry_one() {
    let mut missing = dependency_graph();
    missing
        .nodes
        .iter_mut()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
        .unwrap()
        .attribute_name = None;
    assert!(derive_nisar_subset_dependency_closure_v2(
        &missing,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .is_err());

    for invalid in ["", ".", "..", "grid/mapping", "grid\\mapping"] {
        let mut graph = dependency_graph();
        graph
            .nodes
            .iter_mut()
            .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::Attribute)
            .unwrap()
            .attribute_name = Some(invalid.to_owned());
        assert!(derive_nisar_subset_dependency_closure_v2(
            &graph,
            &NisarSubsetFrequency::A,
            &["HH".to_owned(), "HV".to_owned()],
        )
        .is_err());
    }

    let mut non_attribute = dependency_graph();
    non_attribute
        .nodes
        .iter_mut()
        .find(|node| node.kind == NisarSubsetSourceDependencyNodeKindV2::CoordinateDataset)
        .unwrap()
        .attribute_name = Some("Conventions".to_owned());
    assert!(derive_nisar_subset_dependency_closure_v2(
        &non_attribute,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .is_err());
}

#[test]
fn graph_and_edge_order_do_not_change_graph_or_closure_identity() {
    let graph = dependency_graph();
    let first = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .unwrap();
    let mut reordered = graph;
    reordered.nodes.reverse();
    reordered.edges.reverse();
    let second = derive_nisar_subset_dependency_closure_v2(
        &reordered,
        &NisarSubsetFrequency::A,
        &["HV".to_owned(), "HH".to_owned()],
    )
    .unwrap();
    assert_eq!(first.source_graph_sha256, second.source_graph_sha256);
    assert_eq!(first, second);
}

#[test]
fn graph_content_and_selected_polarization_change_closure_and_plan_identity() {
    let first = plan();
    let old_path = "/science/LSAR/RSLC/metadata/orbit";
    let new_path = "/science/LSAR/RSLC/metadata/orbitStateVectors";
    let mut changed_graph = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    );
    changed_graph
        .source_dependency_graph
        .nodes
        .iter_mut()
        .find(|node| node.source_path == old_path)
        .unwrap()
        .source_path = new_path.to_owned();
    for edge in &mut changed_graph.source_dependency_graph.edges {
        if edge.dependency_path == old_path {
            edge.dependency_path = new_path.to_owned();
        }
    }
    let changed_graph = NisarSubsetPlanV2::build(changed_graph).unwrap();
    assert_ne!(
        first.required_dependencies.source_graph_sha256,
        changed_graph.required_dependencies.source_graph_sha256
    );
    assert_ne!(
        first.canonical_semantic_plan_digest,
        changed_graph.canonical_semantic_plan_digest
    );

    let mut hh_only = draft(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.123456789012345, 2.0, 8.0, 8.0),
    );
    hh_only.selected_polarizations = vec!["HH".to_owned()];
    let hh_only = NisarSubsetPlanV2::build(hh_only).unwrap();
    assert_ne!(first.required_dependencies, hh_only.required_dependencies);
    assert_ne!(
        first.canonical_semantic_plan_digest,
        hh_only.canonical_semantic_plan_digest
    );
}

#[test]
fn missing_coordinate_dimension_or_mask_dependencies_fail_closed() {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    for (kind, relation) in [
        (Kind::CoordinateDataset, Relation::Coordinate),
        (Kind::DimensionScale, Relation::DimensionScale),
        (Kind::ValidityMask, Relation::ValidityMask),
    ] {
        let mut invalid = draft(
            rectangle(0.0, 0.0, 10.0, 10.0),
            rectangle(2.0, 2.0, 8.0, 8.0),
        );
        invalid
            .source_dependency_graph
            .nodes
            .retain(|node| !(node.kind == kind && node.source_path.contains("frequencyA")));
        invalid.source_dependency_graph.edges.retain(|edge| {
            !(edge.relation == relation && edge.dependency_path.contains("frequencyA"))
        });
        assert!(matches!(
            NisarSubsetPlanV2::build(invalid),
            Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(_))
        ));
    }
}

#[test]
fn ssar_scale_less_dependency_graph_is_valid_when_source_has_no_scale_attachments() {
    use NisarSubsetDependencyKindV2 as DependencyKind;
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    let mut graph = dependency_graph();
    graph.nodes.retain(|node| node.kind != Kind::DimensionScale);
    graph
        .edges
        .retain(|edge| edge.relation != Relation::DimensionScale);
    for node in &mut graph.nodes {
        node.source_path = node.source_path.replace("/science/LSAR/", "/science/SSAR/");
    }
    for edge in &mut graph.edges {
        edge.dependent_path = edge
            .dependent_path
            .replace("/science/LSAR/", "/science/SSAR/");
        edge.dependency_path = edge
            .dependency_path
            .replace("/science/LSAR/", "/science/SSAR/");
    }

    let closure = derive_nisar_subset_dependency_closure_v2(
        &graph,
        &NisarSubsetFrequency::A,
        &["HH".to_owned(), "HV".to_owned()],
    )
    .unwrap();

    assert!(!closure
        .items
        .iter()
        .any(|item| item.kind == DependencyKind::DimensionScale));
    for kind in [
        DependencyKind::CoordinateDataset,
        DependencyKind::ValidityMask,
        DependencyKind::IdentificationMetadata,
        DependencyKind::ProvenanceMetadata,
        DependencyKind::Attribute,
        DependencyKind::OtherScientificDependency,
    ] {
        assert!(closure.items.iter().any(|item| item.kind == kind));
    }
}

#[test]
fn unknown_or_orphan_graph_evidence_and_caller_completion_fields_fail_closed() {
    let mut unknown = dependency_graph();
    unknown.nodes[0].kind = NisarSubsetSourceDependencyNodeKindV2::Unknown;
    assert!(matches!(
        derive_nisar_subset_dependency_closure_v2(
            &unknown,
            &NisarSubsetFrequency::A,
            &["HH".to_owned()]
        ),
        Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(_))
    ));

    let mut orphan = dependency_graph();
    orphan.nodes.push(source_node(
        NisarSubsetSourceDependencyNodeKindV2::OtherScientificDependency,
        "/science/LSAR/RSLC/metadata/orphanRequiredNode",
    ));
    assert!(matches!(
        derive_nisar_subset_dependency_closure_v2(
            &orphan,
            &NisarSubsetFrequency::A,
            &["HH".to_owned()]
        ),
        Err(NisarSubsetPlanV2Error::DependencyGraphInvalid(_))
    ));

    let mut caller_claim = serde_json::to_value(dependency_graph()).unwrap();
    caller_claim["complete"] = json!(true);
    caller_claim["unknown_dependency_count"] = json!(0);
    caller_claim["source_graph_sha256"] = json!("f".repeat(64));
    caller_claim["items"] = json!([]);
    assert!(serde_json::from_value::<NisarSubsetSourceDependencyGraphV2>(caller_claim).is_err());
}

#[test]
fn dependency_closure_is_deterministic_and_persisted_tampering_is_rejected() {
    let first = plan();
    let second = plan();
    assert_eq!(first.required_dependencies, second.required_dependencies);
    assert_eq!(
        first.required_dependencies.source_graph_sha256,
        second.required_dependencies.source_graph_sha256
    );

    let mut tampered = first;
    tampered.required_dependencies.complete = false;
    assert!(matches!(
        tampered.validate(),
        Err(NisarSubsetPlanV2Error::DependencyClosureInvalid(_))
    ));
}

#[test]
fn prepared_intent_preserves_phase_one_polygonal_authority_and_normalizes_selection() {
    let with_hole = geometry(json!({
        "type": "Polygon",
        "coordinates": [
            [[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0], [1.0, 1.0]],
            [[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0], [4.0, 4.0]]
        ]
    }));
    let hole = prepared_intent(rectangle(0.0, 0.0, 10.0, 10.0), with_hole);
    assert_eq!(hole.effective_geometry.ring_count, 2);
    assert_eq!(hole.selected_polarizations, ["HH", "HV"]);

    let multi = geometry(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[7.0, -1.0], [9.0, -1.0], [9.0, 2.0], [7.0, 2.0], [7.0, -1.0]]],
            [[[2.0, 2.0], [3.0, 2.0], [3.0, 3.0], [2.0, 3.0], [2.0, 2.0]]]
        ]
    }));
    let partial = prepared_intent(rectangle(0.0, 0.0, 8.0, 8.0), multi);
    assert_eq!(partial.requested_geometry.component_count, 2);
    assert_eq!(partial.effective_geometry.component_count, 2);
    partial.validate().unwrap();
}

#[test]
fn prepared_intent_json_roundtrip_preserves_exact_digest_and_excludes_execution_evidence() {
    let first = prepared_intent(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    let canonical = first.canonical_json().unwrap();
    assert!(!canonical.contains("source_dependency_graph"));
    assert!(!canonical.contains("required_dependencies"));
    assert!(!canonical.contains("warnings"));
    assert!(!canonical.contains("destination"));
    assert!(!canonical.contains("canonical_semantic_plan_digest"));
    let roundtrip: NisarSubsetPreparedIntentV2 = serde_json::from_str(&canonical).unwrap();
    roundtrip.validate().unwrap();
    assert_eq!(
        first.canonical_intent_sha256,
        roundtrip.canonical_intent_sha256
    );
    assert_eq!(canonical, roundtrip.canonical_json().unwrap());
}

#[test]
fn prepared_intent_digest_binds_source_geometry_frequency_and_polarizations() {
    let base = prepared_intent(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    let mut source_change = NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: product_identity(),
        source_asset_identity: asset_identity(),
        source_capability: source_capability(),
        source_footprint: rectangle(0.0, 0.0, 10.0, 10.0),
        requested_geometry: rectangle(2.0, 2.0, 8.0, 8.0),
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HH".to_owned(), "HV".to_owned()],
    };
    source_change
        .source_asset_identity
        .checksum
        .as_mut()
        .unwrap()
        .value = "e".repeat(64);
    let changed_source = NisarSubsetPreparedIntentV2::build(source_change.clone()).unwrap();
    assert_ne!(
        base.canonical_intent_sha256,
        changed_source.canonical_intent_sha256
    );

    source_change.source_asset_identity = asset_identity();
    source_change.source_capability.evidence_sha256 = "f".repeat(64);
    let changed_capability = NisarSubsetPreparedIntentV2::build(source_change.clone()).unwrap();
    assert_ne!(
        base.canonical_intent_sha256,
        changed_capability.canonical_intent_sha256
    );

    source_change.source_capability = source_capability();
    source_change.requested_geometry = rectangle(3.0, 2.0, 8.0, 8.0);
    let changed_geometry = NisarSubsetPreparedIntentV2::build(source_change.clone()).unwrap();
    assert_ne!(
        base.canonical_intent_sha256,
        changed_geometry.canonical_intent_sha256
    );

    let mut frequency_a_input = NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: product_identity(),
        source_asset_identity: asset_identity(),
        source_capability: source_capability(),
        source_footprint: rectangle(0.0, 0.0, 10.0, 10.0),
        requested_geometry: rectangle(2.0, 2.0, 8.0, 8.0),
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HH".to_owned()],
    };
    frequency_a_input
        .source_capability
        .frequencies
        .iter_mut()
        .find(|capability| capability.frequency == NisarSubsetFrequency::B)
        .unwrap()
        .available_polarizations = vec!["HH".to_owned(), "HV".to_owned()];
    let frequency_a = NisarSubsetPreparedIntentV2::build(frequency_a_input.clone()).unwrap();
    let mut frequency_b_input = frequency_a_input.clone();
    frequency_b_input.selected_frequency = NisarSubsetFrequency::B;
    let frequency_b = NisarSubsetPreparedIntentV2::build(frequency_b_input).unwrap();
    assert_ne!(
        frequency_a.canonical_intent_sha256,
        frequency_b.canonical_intent_sha256
    );

    let mut polarization_hv_input = frequency_a_input;
    polarization_hv_input.selected_polarizations = vec!["HV".to_owned()];
    let polarization_hv = NisarSubsetPreparedIntentV2::build(polarization_hv_input).unwrap();
    assert_ne!(
        frequency_a.canonical_intent_sha256,
        polarization_hv.canonical_intent_sha256
    );
}

#[test]
fn prepared_intent_antimeridian_fails_closed() {
    let crossing = geometry(json!({
        "type": "Polygon",
        "coordinates": [[[179.0, -1.0], [-179.0, -1.0], [-179.0, 1.0], [179.0, 1.0], [179.0, -1.0]]]
    }));
    assert_eq!(
        NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
            source_product_identity: product_identity(),
            source_asset_identity: asset_identity(),
            source_capability: source_capability(),
            source_footprint: rectangle(-10.0, -10.0, 10.0, 10.0),
            requested_geometry: crossing,
            selected_frequency: NisarSubsetFrequency::A,
            selected_polarizations: vec!["HH".to_owned()],
        }),
        Err(NisarSubsetPlanV2Error::AntimeridianUnsupported {
            geometry: NisarSubsetGeometryRoleV2::Requested,
        })
    );
}

#[test]
fn final_plan_is_exactly_bound_to_prepared_intent_while_warnings_are_not_intent_identity() {
    let intent = prepared_intent(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(2.0, 2.0, 8.0, 8.0),
    );
    let first = NisarSubsetPlanV2::build_from_prepared_intent(
        intent.clone(),
        dependency_graph(),
        metadata_evidence(vec!["warning one".to_owned()]),
    )
    .unwrap();
    let second = NisarSubsetPlanV2::build_from_prepared_intent(
        intent.clone(),
        dependency_graph(),
        metadata_evidence(vec!["warning two".to_owned()]),
    )
    .unwrap();
    assert_eq!(first.prepared_intent_sha256, intent.canonical_intent_sha256);
    assert_eq!(
        second.prepared_intent_sha256,
        intent.canonical_intent_sha256
    );
    assert_eq!(first.prepared_intent_sha256, second.prepared_intent_sha256);

    let other = prepared_intent(
        rectangle(0.0, 0.0, 10.0, 10.0),
        rectangle(3.0, 2.0, 8.0, 8.0),
    );
    let mut tampered = first;
    tampered.prepared_intent_sha256 = other.canonical_intent_sha256;
    assert!(matches!(
        tampered.validate(),
        Err(NisarSubsetPlanV2Error::IdentityMismatch(_))
    ));
}
