use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};

use insar_subset_lab::{
    validate_reference_window, Window2D, SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO,
};

const DONOR_FIXTURE_SHA256: &str =
    "40d7d2c6ddc96730805666fdea76f465cd87dfff1afba7813df8b9b8e5eeaf14";

#[test]
fn pinned_official_single_scene_evidence_preserves_donor_contract_and_recomputable_fields() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/fixtures/asf-nisar-subset-p0/official-single-scene-result.json");
    let bytes = fs::read(path).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        DONOR_FIXTURE_SHA256
    );
    let evidence: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(evidence["schema"], "NisarOfficialSingleSceneEvidenceV1");
    assert_eq!(
        evidence["source"]["granule_id"],
        "NISAR_L1_PR_RSLC_026_093_D_083_2005_DHDH_A_20260726T155803_20260726T155837_P05023_N_F_J_001"
    );
    assert_eq!(evidence["source"]["size_bytes"], 14_252_244_992_u64);
    assert_eq!(
        evidence["source"]["sha256"],
        "ee3de9b9877b4f8e35153369725dcb4174ef5f2423af167e77f843e08860a9a0"
    );
    assert_eq!(evidence["source"]["product_specification_version"], "1.5.0");

    let reference = &evidence["independent_reference"];
    assert_eq!(reference["engine"], "isce3.geometry.geo2rdr_bracket");
    assert_eq!(reference["isce3_version"], "0.25.12");
    assert_eq!(reference["radar_grid_doppler"], "zero_hz");
    assert_eq!(reference["product_doppler_centroid_used"], false);
    assert_eq!(
        reference["densification_degrees"].as_array().unwrap().len(),
        3
    );
    assert_eq!(reference["all_logical_and_chunk_windows_stable"], true);
    assert_eq!(
        reference["benchmark_family_sha256"],
        "3d2d4c8ac55d67f64bfeaac619584bd00f3d20eace6e87bd13cc6e4446ca45c0"
    );

    let legacy = &evidence["geometry_gate"]["legacy_bbox_nodes"];
    assert_eq!(legacy["passed"], false);
    assert_eq!(
        legacy["false_no_overlap_cases"].as_array().unwrap().len(),
        2
    );
    assert_eq!(legacy["undercoverage_cases"].as_array().unwrap().len(), 1);
    let hardened = &evidence["geometry_gate"]["hardened_cells_halo0"];
    assert_eq!(hardened["passed"], true);
    assert_eq!(hardened["zero_false_no_overlap"], true);
    assert_eq!(hardened["zero_undercoverage"], true);
    let observed_area = hardened["maximum_candidate_to_reference_chunk_area_ratio"]
        .as_f64()
        .unwrap();
    let admitted_area = hardened["admission_area_ratio_maximum"].as_f64().unwrap();
    assert_eq!(
        admitted_area,
        SPARSE_GRID_MAX_CANDIDATE_REFERENCE_CHUNK_AREA_RATIO
    );
    assert!(observed_area <= admitted_area);
    assert!((observed_area - 4.0 / 3.0).abs() < 1.0e-12);
    assert_eq!(
        evidence["geometry_gate"]["selected_method"],
        "sparse_grid_polygon_cells_v1"
    );
    assert_eq!(evidence["geometry_gate"]["selected_chunk_halo"], 0);
    for (frequency, shape) in [("A", [53_200, 26_369]), ("B", [53_200, 6_593])] {
        let window = &evidence["selection"]["windows"][frequency];
        let emitted = Window2D::new(
            window["row_start"].as_u64().unwrap() as usize,
            window["row_stop"].as_u64().unwrap() as usize,
            window["column_start"].as_u64().unwrap() as usize,
            window["column_stop"].as_u64().unwrap() as usize,
            shape,
        )
        .unwrap();
        assert_eq!(validate_reference_window(emitted, emitted).unwrap(), 1.0);
    }
    let canonical_aoi = serde_json::to_vec(&evidence["selection"]["requested_aoi"]).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(canonical_aoi)),
        evidence["selection"]["requested_aoi_sha256"]
    );

    let science = &evidence["science"];
    assert_eq!(science["datasets"].as_array().unwrap().len(), 4);
    assert_eq!(science["qa_masks"].as_array().unwrap().len(), 2);
    assert_eq!(science["reference_local_array_equal"], true);
    assert_eq!(science["reference_range_array_equal"], true);
    assert_eq!(science["all_mismatch_counts"], 0);
    assert_eq!(science["max_complex_error"], 0.0);
    assert_eq!(science["max_amplitude_error"], 0.0);
    assert_eq!(science["max_wrapped_phase_error"], 0.0);

    let metadata = &evidence["metadata"];
    assert_eq!(metadata["source_dataset_path_count"], 207);
    assert_eq!(metadata["all_source_dataset_paths_present"], true);
    assert_eq!(metadata["metadata_complete_local"], true);
    assert_eq!(metadata["metadata_complete_range"], true);
    assert_eq!(metadata["science_statistics_scope"], "derived_subset");
    assert_eq!(metadata["dimension_scales_verified"], true);

    let output = &evidence["output"];
    assert_eq!(output["local_size_bytes"], 173_806_009_u64);
    assert_eq!(output["range_size_bytes"], 173_806_009_u64);
    assert_eq!(output["local_sha256"], output["range_sha256"]);
    assert_eq!(
        output["range_sha256"],
        "3c2c408ff8fab29cc1035eab2a19f4bd7b6496af13f53bf8f4aaf64a61e23912"
    );
    assert_eq!(output["byte_for_byte_equal"], true);
    assert_eq!(output["atomic_promotion"], true);
    assert_eq!(output["validated_before_promotion"], true);

    let transfer = &evidence["transfer"];
    let received = transfer["actual_received_bytes"].as_u64().unwrap();
    let full = transfer["full_product_bytes"].as_u64().unwrap();
    let ratio = received as f64 / full as f64;
    assert_eq!(transfer["request_count"], 192);
    assert_eq!(transfer["all_responses_206"], true);
    assert_eq!(transfer["all_requests_used_if_range"], true);
    assert_eq!(received, 198_995_157);
    assert_eq!(full, 14_252_244_992);
    assert!((ratio - transfer["transfer_full_ratio"].as_f64().unwrap()).abs() < 1.0e-15);
    assert!((1.0 - ratio - transfer["savings_ratio"].as_f64().unwrap()).abs() < 1.0e-15);
    assert!(ratio < transfer["budget_full_ratio_maximum"].as_f64().unwrap());
    assert_eq!(transfer["budget_passed"], true);

    let conclusion = &evidence["conclusion"];
    assert_eq!(conclusion["implementation"], "research_prototype");
    assert_eq!(conclusion["single_scene_validation"], "passed");
    assert_eq!(conclusion["interferometric_validation"], "not_started");
    assert_eq!(conclusion["production_verification"], "not_verified");
}
