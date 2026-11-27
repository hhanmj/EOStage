use std::fs;
use std::path::PathBuf;

use serde_json::Value;

#[test]
fn p0_5_no_go_is_machine_readable_and_does_not_authorize_p1() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/fixtures/asf-nisar-subset-p0/p0-5-entry-decision.json");
    let decision: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

    assert_eq!(
        decision["fixture_contract"],
        "NisarSubsetP05EntryDecisionV1"
    );
    assert_eq!(decision["decision"], "NO_GO");
    assert_eq!(decision["research_feasibility"], "GO");
    assert_eq!(decision["p1_entry_authorized"], false);
    assert_eq!(decision["production_enablement"], false);
    assert_eq!(decision["scope"]["artifact_authority"], "client_derived");

    let boundary = &decision["validation_boundary"];
    assert_eq!(boundary["implementation"], "research_prototype");
    assert_eq!(boundary["single_scene_validation"], "passed");
    assert_eq!(boundary["interferometric_validation"], "not_started");
    assert_eq!(boundary["production_verification"], "not_verified");

    let conditions = decision["entry_conditions"].as_array().unwrap();
    assert_eq!(conditions.len(), 9);
    assert!(conditions.iter().all(|condition| matches!(
        condition["state"].as_str().unwrap(),
        "satisfied" | "partially_satisfied" | "unsatisfied"
    )));
    assert!(conditions.iter().any(|condition| {
        condition["entry_blocking"] == true && condition["state"] != "satisfied"
    }));
    assert_eq!(decision["root_entry_blockers"].as_array().unwrap().len(), 4);

    let evidence = &decision["research_evidence"];
    assert_eq!(
        evidence["donor_commit"],
        "871e4322ba4ec39d0afb7aa8397f81d663acb392"
    );
    assert_eq!(
        evidence["fixture_sha256"],
        "40d7d2c6ddc96730805666fdea76f465cd87dfff1afba7813df8b9b8e5eeaf14"
    );
    let received = evidence["actual_received_bytes"].as_u64().unwrap();
    let full = evidence["full_product_bytes"].as_u64().unwrap();
    assert!(
        (received as f64 / full as f64 - evidence["transfer_full_ratio"].as_f64().unwrap()).abs()
            < 1.0e-15
    );

    assert_eq!(decision["current_authorized_tree_review"]["p0"], 0);
    assert_eq!(decision["current_authorized_tree_review"]["p1"], 0);
    assert_eq!(decision["complete_product_path_preserved"], true);
}
