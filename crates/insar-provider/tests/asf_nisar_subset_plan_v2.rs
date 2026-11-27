use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, FrozenAssetAccess,
    NisarSubsetDependencyEvidenceAuthorityV2, NisarSubsetDependencyRelationV2,
    NisarSubsetFrequency, NisarSubsetSourceDependencyEdgeV2, NisarSubsetSourceDependencyGraphV2,
    NisarSubsetSourceDependencyNodeKindV2, NisarSubsetSourceDependencyNodeV2, SnapshotItem,
    StableAssetReference, NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use insar_provider::asf::nisar_subset::{
    canonical_nisar_polarizations_for_frequency_label, plan_nisar_subset_v2,
    prepare_nisar_subset_intent_v2, AsfNisarSubsetPlanDraftV2, AsfNisarSubsetPreparedIntentDraftV2,
};
use serde_json::{json, Map, Value};

const FILE_NAME: &str = "NISAR_L1_PR_RSLC_TEST_P05023.h5";
const PRODUCT_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF";
const ASSET_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5";

fn geometry(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn rectangle(west: f64, south: f64, east: f64, north: f64) -> Map<String, Value> {
    geometry(json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    }))
}

fn snapshot() -> SnapshotItem {
    SnapshotItem {
        item_id: PRODUCT_KEY.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: FILE_NAME.trim_end_matches(".h5").to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(rectangle(36.0, 9.0, 39.0, 12.0)),
        metadata: Map::from_iter([
            ("band".to_owned(), json!("L")),
            ("processing_level".to_owned(), json!("RSLC")),
            ("science_level".to_owned(), json!("L1")),
            ("quality_status".to_owned(), json!("Provisional")),
            ("production_configuration".to_owned(), json!("PR")),
            ("crid".to_owned(), json!("P05023")),
            ("product_specification_version".to_owned(), json!("1.5.0")),
            (
                "collection_profile_id".to_owned(),
                json!("asf.nisar.l.rslc.provisional.v1"),
            ),
            ("frequency_a_polarization".to_owned(), json!("HH+HV+VH+VV")),
            ("frequency_b_polarization".to_owned(), json!("VV+VH")),
        ]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: "asf.nasa.earthdata".to_owned(),
                collection_id: "C2853145197-ASF".to_owned(),
                provider_product_id: "G9900000001-ASF".to_owned(),
                provider_product_key: PRODUCT_KEY.to_owned(),
                canonical_product_key: None,
                asset_key: "science-rslc-hdf5".to_owned(),
                stable_asset_key: ASSET_KEY.to_owned(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: "earthdata_https".to_owned(),
                scheme: AssetAccessScheme::EarthdataHttps,
                stable_href: None,
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(FILE_NAME.to_owned()),
            expected_size_bytes: Some(14_252_244_992),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("e".repeat(64)),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
        }),
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

    let hh = "/science/LSAR/RSLC/swaths/frequencyA/HH";
    let hv = "/science/LSAR/RSLC/swaths/frequencyA/HV";
    let coordinate = "/science/LSAR/RSLC/metadata/geolocationGrid/frequencyA/coordinateX";
    let dimension = "/science/LSAR/RSLC/swaths/frequencyA/zeroDopplerTime";
    let mask = "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask";
    let identification = "/science/LSAR/identification/granuleId";
    let provenance = "/science/LSAR/identification/productVersion";
    let attribute = "/";
    let other = "/science/LSAR/RSLC/metadata/orbit";
    let mut nodes = vec![
        source_node(Kind::ScienceDataset, hh),
        source_node(Kind::ScienceDataset, hv),
        source_node(Kind::CoordinateDataset, coordinate),
        source_node(Kind::DimensionScale, dimension),
        source_node(Kind::ValidityMask, mask),
        source_node(Kind::IdentificationMetadata, identification),
        source_node(Kind::ProvenanceMetadata, provenance),
        source_node(Kind::Attribute, attribute),
        source_node(Kind::OtherScientificDependency, other),
    ];
    let mut edges = Vec::new();
    for science in [hh, hv] {
        edges.extend([
            source_edge(science, coordinate, Relation::Coordinate),
            source_edge(science, dimension, Relation::DimensionScale),
            source_edge(science, mask, Relation::ValidityMask),
            source_edge(science, identification, Relation::RequiredMetadata),
            source_edge(science, provenance, Relation::Provenance),
            source_edge(science, attribute, Relation::RequiredAttribute),
            source_edge(science, other, Relation::OtherScientificDependency),
        ]);
    }
    nodes.reverse();
    edges.reverse();
    NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2::Hdf5MetadataInventoryV1,
        evidence_sha256: "f".repeat(64),
        nodes,
        edges,
    }
}

fn draft(requested_geometry: Map<String, Value>) -> AsfNisarSubsetPlanDraftV2 {
    AsfNisarSubsetPlanDraftV2 {
        requested_geometry,
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HV".to_owned(), "HH".to_owned(), "HH".to_owned()],
        source_dependency_graph: dependency_graph(),
        transfer_estimate: None,
        derived_estimate: None,
        warnings: Vec::new(),
    }
}

#[test]
fn provider_prepares_intent_without_dependency_graph_and_preserves_source_projection() {
    let item = snapshot();
    let before = item.clone();
    let intent = prepare_nisar_subset_intent_v2(
        &item,
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: rectangle(37.0, 10.0, 38.0, 11.0),
            selected_frequency: NisarSubsetFrequency::A,
            selected_polarizations: vec!["HV".to_owned(), "HH".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(item, before);
    assert_eq!(intent.selected_polarizations, ["HH", "HV"]);
    assert_eq!(
        intent.source_product_identity.provider_product_key,
        PRODUCT_KEY
    );
    assert_eq!(intent.source_asset_identity.stable_asset_key, ASSET_KEY);
    intent.validate().unwrap();
    let round_trip: insar_core::NisarSubsetPreparedIntentV2 =
        serde_json::from_str(&intent.canonical_json().unwrap()).unwrap();
    assert_eq!(round_trip, intent);
}

#[test]
fn provider_accepts_catalog_valid_vh_and_a_vv_and_canonicalizes_duplicates() {
    let intent = prepare_nisar_subset_intent_v2(
        &snapshot(),
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: rectangle(37.0, 10.0, 38.0, 11.0),
            selected_frequency: NisarSubsetFrequency::A,
            selected_polarizations: vec!["VV".to_owned(), "VH".to_owned(), "VV".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(intent.selected_polarizations, ["VH", "VV"]);
    intent.validate().unwrap();
}

#[test]
fn provider_accepts_frequency_b_vv_vh_and_rejects_unsorted_profile() {
    let item = snapshot();
    let intent = prepare_nisar_subset_intent_v2(
        &item,
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: rectangle(37.0, 10.0, 38.0, 11.0),
            selected_frequency: NisarSubsetFrequency::B,
            selected_polarizations: vec!["VV".to_owned(), "VH".to_owned()],
        },
    )
    .unwrap();

    assert_eq!(intent.selected_frequency, NisarSubsetFrequency::B);
    assert_eq!(intent.selected_polarizations, ["VH", "VV"]);
    let frequency_b = intent
        .source_capability
        .frequencies
        .iter()
        .find(|capability| capability.frequency == NisarSubsetFrequency::B)
        .expect("Frequency B capability");
    assert_eq!(frequency_b.available_polarizations, ["VH", "VV"]);
    assert_eq!(
        canonical_nisar_polarizations_for_frequency_label(&NisarSubsetFrequency::B, "VV+VH"),
        Some(vec!["VH".to_owned(), "VV".to_owned()])
    );

    for label in ["VH+VV", "VV+VV", "HH+VV"] {
        assert_eq!(
            canonical_nisar_polarizations_for_frequency_label(&NisarSubsetFrequency::B, label),
            None,
            "invalid Frequency B profile label: {label}"
        );
    }

    let mut invalid = item;
    invalid
        .metadata
        .insert("frequency_b_polarization".to_owned(), json!("VH+VV"));
    assert!(prepare_nisar_subset_intent_v2(
        &invalid,
        AsfNisarSubsetPreparedIntentDraftV2 {
            requested_geometry: rectangle(37.0, 10.0, 38.0, 11.0),
            selected_frequency: NisarSubsetFrequency::B,
            selected_polarizations: vec!["VV".to_owned(), "VH".to_owned()],
        },
    )
    .is_err());
}

#[test]
fn provider_projects_actual_source_capability_without_mutating_snapshot() {
    let item = snapshot();
    let before = item.clone();
    let plan = plan_nisar_subset_v2(&item, draft(rectangle(37.0, 10.0, 38.0, 11.0))).unwrap();

    assert_eq!(item, before);
    assert_eq!(plan.selected_polarizations, ["HH", "HV"]);
    assert_eq!(plan.source_capability.frequencies.len(), 2);
    assert_eq!(
        plan.source_capability.frequencies[0].available_polarizations,
        ["HH", "HV", "VH", "VV"]
    );
    assert_eq!(
        plan.source_capability.frequencies[1].available_polarizations,
        ["VH", "VV"]
    );
    assert_eq!(
        serde_json::to_value(plan.source_product_identity.quality_status).unwrap(),
        json!("PROVISIONAL")
    );
    assert_eq!(plan.source_estimate.as_ref().unwrap().bytes, 14_252_244_992);
    assert_eq!(plan.source_footprint_geometry.crs, "EPSG:4326");
    assert_eq!(plan.source_footprint_geometry.component_count, 1);
    assert_eq!(plan.source_footprint_sha256.len(), 64);
    assert_eq!(
        plan.provenance.source_footprint_sha256,
        plan.source_footprint_sha256
    );
    assert!(plan.required_dependencies.complete);
    assert_eq!(plan.required_dependencies.unknown_dependency_count, 0);
    assert_ne!(
        plan.required_dependencies.source_graph_sha256,
        plan.source_dependency_graph.evidence_sha256
    );
    plan.validate().unwrap();
}

#[test]
fn provider_rejects_polarization_absent_from_actual_source() {
    let mut item = snapshot();
    item.metadata
        .insert("frequency_a_polarization".to_owned(), json!("HH+HV"));
    let mut request = draft(rectangle(37.0, 10.0, 38.0, 11.0));
    request.selected_polarizations = vec!["VV".to_owned()];
    assert_eq!(
        plan_nisar_subset_v2(&item, request).unwrap_err().code,
        "NISAR_SUBSET_POLARIZATION_UNSUPPORTED"
    );
}

#[test]
fn provider_preserves_typed_antimeridian_and_dependency_failures() {
    let crossing = geometry(json!({
        "type": "Polygon",
        "coordinates": [[[179.0, -1.0], [-179.0, -1.0], [-179.0, 1.0], [179.0, 1.0], [179.0, -1.0]]]
    }));
    assert_eq!(
        plan_nisar_subset_v2(&snapshot(), draft(crossing))
            .unwrap_err()
            .code,
        "NISAR_SUBSET_ANTIMERIDIAN_UNSUPPORTED"
    );

    let mut request = draft(rectangle(37.0, 10.0, 38.0, 11.0));
    request.source_dependency_graph.nodes[0].kind = NisarSubsetSourceDependencyNodeKindV2::Unknown;
    assert_eq!(
        plan_nisar_subset_v2(&snapshot(), request).unwrap_err().code,
        "NISAR_SUBSET_DEPENDENCY_CLOSURE_INVALID"
    );
}

#[test]
fn provider_requires_official_provisional_quality_status() {
    let mut item = snapshot();
    item.metadata
        .insert("quality_status".to_owned(), json!("Beta"));
    assert_eq!(
        plan_nisar_subset_v2(&item, draft(rectangle(37.0, 10.0, 38.0, 11.0)))
            .unwrap_err()
            .code,
        "NISAR_SUBSET_NOT_SUPPORTED"
    );
}
