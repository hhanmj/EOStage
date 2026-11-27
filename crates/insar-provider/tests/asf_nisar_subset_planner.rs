use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, NisarSubsetFrequency, SnapshotItem,
    StableAssetReference,
};
use insar_provider::asf::nisar_subset::{
    normalize_nisar_subset_aoi, plan_nisar_subset, NisarSubsetEvidenceWindowV1,
    NisarSubsetGeolocationEvidenceV1,
};
use serde_json::{json, Map, Value};
use std::fs;

const FILE_NAME: &str =
    "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001.h5";
const PRODUCT_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF";
const ASSET_KEY: &str =
    "PROVIDER::asf.nasa.earthdata::COLLECTION::C2853145197-ASF::ITEM::G9900000001-ASF::ASSET::science-rslc-hdf5";

fn official_aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.85690570018474],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn snapshot() -> SnapshotItem {
    SnapshotItem {
        item_id: PRODUCT_KEY.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: FILE_NAME.trim_end_matches(".h5").to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(
            json!({
                "type": "Polygon",
                "coordinates": [[[36.0, 9.0], [39.0, 9.0], [39.0, 12.0], [36.0, 12.0], [36.0, 9.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        metadata: Map::from_iter([
            ("band".to_owned(), json!("L")),
            ("processing_level".to_owned(), json!("RSLC")),
            ("science_level".to_owned(), json!("L1")),
            ("production_configuration".to_owned(), json!("PR")),
            ("crid".to_owned(), json!("P05023")),
            ("product_specification_version".to_owned(), json!("1.5.0")),
            (
                "collection_profile_id".to_owned(),
                json!("asf.nisar.l.rslc.provisional.v1"),
            ),
            ("frequency_a_polarization".to_owned(), json!("HH+HV")),
            ("frequency_b_polarization".to_owned(), json!("HH+HV")),
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
            checksum_value: Some(
                "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0".to_owned(),
            ),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some("asf.nisar.l.rslc.provisional.v1".to_owned()),
        }),
    }
}

fn evidence(
    aoi: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
    windows: Vec<NisarSubsetEvidenceWindowV1>,
) -> NisarSubsetGeolocationEvidenceV1 {
    let normalized = normalize_nisar_subset_aoi(aoi).unwrap();
    NisarSubsetGeolocationEvidenceV1 {
        aoi_sha256: normalized.sha256,
        algorithm: "sparse_grid_polygon_cells_v1".to_owned(),
        algorithm_version: 1,
        evidence_digest: "3d2d4c8ac55d67f64bfeaac619584bd00f3d20eace6e87bd13cc6e4446ca45c0"
            .to_owned(),
        product_specification_version: "1.5.0".to_owned(),
        crid: "P05023".to_owned(),
        frequency,
        swath: "LSAR_RSLC".to_owned(),
        source_shape: [70_000, 30_000],
        chunk_shape: [512, 512],
        dtype: "CFloat32".to_owned(),
        windows,
        estimated_remote_bytes: 198_995_157,
        estimated_output_bytes: 173_806_009,
    }
}

fn window(component_id: u16, frequency: NisarSubsetFrequency) -> NisarSubsetEvidenceWindowV1 {
    let sample_start = match frequency {
        NisarSubsetFrequency::A => 12_800,
        NisarSubsetFrequency::B => 3_072,
    };
    NisarSubsetEvidenceWindowV1 {
        component_id,
        line_start: 26_112 + u64::from(component_id) * 512,
        line_end_exclusive: 26_624 + u64::from(component_id) * 512,
        sample_start,
        sample_end_exclusive: sample_start + 512,
        margin_lines: 0,
        margin_samples: 0,
    }
}

fn plan(
    item: &SnapshotItem,
    aoi: &Map<String, Value>,
    frequency: NisarSubsetFrequency,
    polarization: &str,
    evidence: &NisarSubsetGeolocationEvidenceV1,
) -> Result<insar_core::NisarSubsetRequestManifestV1, insar_core::ContractError> {
    plan_nisar_subset(item, aoi, frequency, polarization, "LSAR_RSLC", evidence)
}

#[test]
fn planner_is_deterministic_immutable_and_uses_dataset_level_polarizations() {
    let aoi = official_aoi();
    for (frequency, polarization, search_label) in [
        (NisarSubsetFrequency::A, "HH", "HH+HV"),
        (NisarSubsetFrequency::A, "HV", "HH+HV"),
        (NisarSubsetFrequency::B, "HH", "HH+HV"),
        (NisarSubsetFrequency::B, "HV", "HH+HV"),
        (NisarSubsetFrequency::B, "VV", "VV"),
    ] {
        let mut item = snapshot();
        item.metadata.insert(
            match frequency {
                NisarSubsetFrequency::A => "frequency_a_polarization",
                NisarSubsetFrequency::B => "frequency_b_polarization",
            }
            .to_owned(),
            json!(search_label),
        );
        let before = item.clone();
        let evidence = evidence(&aoi, frequency.clone(), vec![window(0, frequency.clone())]);
        let first = plan(&item, &aoi, frequency.clone(), polarization, &evidence).unwrap();
        let second = plan(&item, &aoi, frequency.clone(), polarization, &evidence).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.sha256().unwrap(), second.sha256().unwrap());
        assert_eq!(first.request.polarization, polarization);
        assert_eq!(
            first.mapping.windows[0].dataset_path,
            format!(
                "/science/LSAR/RSLC/swaths/frequency{}/{polarization}",
                match frequency {
                    NisarSubsetFrequency::A => "A",
                    NisarSubsetFrequency::B => "B",
                }
            )
        );
        assert!(first.mapping.windows[0].validity_mask_required);
        assert_eq!(item, before);
        if frequency == NisarSubsetFrequency::A && polarization == "HH" {
            let fixture: insar_core::NisarSubsetRequestManifestV1 = serde_json::from_str(
                &fs::read_to_string(format!(
                    "{}/../../docs/fixtures/asf-nisar-subset-p1/planner-request-v1.json",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(first, fixture);
        }
    }
}

#[test]
fn planner_normalizes_component_and_evidence_input_order() {
    let aoi = json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[37.73, 10.86], [37.731, 10.86], [37.731, 10.861], [37.73, 10.861], [37.73, 10.86]]],
            [[[37.72, 10.85], [37.721, 10.85], [37.721, 10.851], [37.72, 10.851], [37.72, 10.85]]]
        ]
    })
    .as_object()
    .unwrap()
    .clone();
    let reversed = json!({
        "coordinates": [
            [[[37.72, 10.85], [37.721, 10.85], [37.721, 10.851], [37.72, 10.851], [37.72, 10.85]]],
            [[[37.73, 10.86], [37.731, 10.86], [37.731, 10.861], [37.73, 10.861], [37.73, 10.86]]]
        ],
        "type": "MultiPolygon"
    })
    .as_object()
    .unwrap()
    .clone();
    let normalized = normalize_nisar_subset_aoi(&aoi).unwrap();
    let normalized_reversed = normalize_nisar_subset_aoi(&reversed).unwrap();
    assert_eq!(normalized.sha256, normalized_reversed.sha256);
    assert_eq!(normalized.component_count, 2);

    let mut evidence = evidence(
        &aoi,
        NisarSubsetFrequency::A,
        vec![
            window(1, NisarSubsetFrequency::A),
            window(0, NisarSubsetFrequency::A),
        ],
    );
    let first = plan(&snapshot(), &aoi, NisarSubsetFrequency::A, "HH", &evidence).unwrap();
    evidence.aoi_sha256 = normalized_reversed.sha256;
    evidence.windows.reverse();
    let second = plan(
        &snapshot(),
        &reversed,
        NisarSubsetFrequency::A,
        "HH",
        &evidence,
    )
    .unwrap();
    assert_eq!(first.sha256().unwrap(), second.sha256().unwrap());
    assert_eq!(
        first
            .mapping
            .windows
            .iter()
            .map(|window| window.component_id)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
}

#[test]
fn normalized_aoi_identity_survives_json_persistence_roundtrip() {
    let aoi = json!({
        "type": "Polygon",
        "coordinates": [[
            [37.720222204639484, 10.857787104635449],
            [37.721296026021705, 10.857533983552843],
            [37.721144257142576, 10.85690570018474],
            [37.720070447767895, 10.857158813483217],
            [37.720222204639484, 10.857787104635449]
        ]]
    })
    .as_object()
    .unwrap()
    .clone();

    let first = normalize_nisar_subset_aoi(&aoi).unwrap();
    let persisted = serde_json::to_string(&first.canonical_geojson).unwrap();
    let restored: Map<String, Value> = serde_json::from_str(&persisted).unwrap();
    let second = normalize_nisar_subset_aoi(&restored).unwrap();

    assert_eq!(first.sha256, second.sha256);
    assert_eq!(first.canonical_geojson, second.canonical_geojson);
}

#[test]
fn planner_digest_changes_with_aoi_frequency_polarization_or_source_identity() {
    let aoi = official_aoi();
    let item = snapshot();
    let base_evidence = evidence(
        &aoi,
        NisarSubsetFrequency::A,
        vec![window(0, NisarSubsetFrequency::A)],
    );
    let base = plan(&item, &aoi, NisarSubsetFrequency::A, "HH", &base_evidence)
        .unwrap()
        .sha256()
        .unwrap();

    let mut shifted_aoi = aoi.clone();
    shifted_aoi["coordinates"][0][0][0] = json!(37.72021);
    shifted_aoi["coordinates"][0][4][0] = json!(37.72021);
    let shifted_evidence = evidence(
        &shifted_aoi,
        NisarSubsetFrequency::A,
        vec![window(0, NisarSubsetFrequency::A)],
    );
    assert_ne!(
        base,
        plan(
            &item,
            &shifted_aoi,
            NisarSubsetFrequency::A,
            "HH",
            &shifted_evidence
        )
        .unwrap()
        .sha256()
        .unwrap()
    );

    let b_evidence = evidence(
        &aoi,
        NisarSubsetFrequency::B,
        vec![window(0, NisarSubsetFrequency::B)],
    );
    assert_ne!(
        base,
        plan(&item, &aoi, NisarSubsetFrequency::B, "HH", &b_evidence)
            .unwrap()
            .sha256()
            .unwrap()
    );
    assert_ne!(
        base,
        plan(&item, &aoi, NisarSubsetFrequency::A, "HV", &base_evidence)
            .unwrap()
            .sha256()
            .unwrap()
    );

    let mut changed_source = item.clone();
    changed_source.asset_access.as_mut().unwrap().checksum_value = Some("0".repeat(64));
    assert_ne!(
        base,
        plan(
            &changed_source,
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            &base_evidence
        )
        .unwrap()
        .sha256()
        .unwrap()
    );
}

#[test]
fn planner_fails_closed_for_unsupported_identity_scope_and_mapping() {
    let aoi = official_aoi();
    let base_evidence = evidence(
        &aoi,
        NisarSubsetFrequency::A,
        vec![window(0, NisarSubsetFrequency::A)],
    );
    let unsupported = |mut item: SnapshotItem, field: &str, value: Value| {
        item.metadata.insert(field.to_owned(), value);
        let error = plan(&item, &aoi, NisarSubsetFrequency::A, "HH", &base_evidence).unwrap_err();
        assert_eq!(error.code, "NISAR_SUBSET_NOT_SUPPORTED");
    };
    unsupported(snapshot(), "band", json!("S"));
    unsupported(snapshot(), "crid", json!("P99999"));
    unsupported(snapshot(), "product_specification_version", json!("9.9.9"));
    unsupported(
        snapshot(),
        "collection_profile_id",
        json!("asf.nisar.unknown"),
    );

    let mut bhoonidhi = snapshot();
    bhoonidhi.provider_id = "bhoonidhi.nrsc.gov.in".to_owned();
    assert_eq!(
        plan(
            &bhoonidhi,
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            &base_evidence
        )
        .unwrap_err()
        .code,
        "NISAR_SUBSET_NOT_SUPPORTED"
    );
    assert_eq!(
        plan(
            &snapshot(),
            &aoi,
            NisarSubsetFrequency::A,
            "VV",
            &base_evidence
        )
        .unwrap_err()
        .code,
        "NISAR_SUBSET_NOT_SUPPORTED"
    );

    let mut no_overlap = snapshot();
    no_overlap.footprint_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    assert_eq!(
        plan(
            &no_overlap,
            &aoi,
            NisarSubsetFrequency::A,
            "HH",
            &base_evidence
        )
        .unwrap_err()
        .code,
        "NISAR_SUBSET_MAPPING_UNAVAILABLE"
    );

    let mut missing = base_evidence.clone();
    missing.windows.clear();
    assert_eq!(
        plan(&snapshot(), &aoi, NisarSubsetFrequency::A, "HH", &missing)
            .unwrap_err()
            .code,
        "NISAR_SUBSET_MAPPING_UNAVAILABLE"
    );

    let oversized = json!({
        "type": "Polygon",
        "coordinates": [[[30.0, 5.0], [40.0, 5.0], [40.0, 15.0], [30.0, 15.0], [30.0, 5.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    assert_eq!(
        normalize_nisar_subset_aoi(&oversized).unwrap_err().code,
        "NISAR_SUBSET_MAPPING_UNAVAILABLE"
    );
}
