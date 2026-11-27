use std::fs;
use std::path::PathBuf;

use insar_core::{
    match_orbits, parse_orbit_filenames, OrbitMatchReport, OrbitMatchRequest, OrbitParseRequest,
    OrbitParseResponse,
};
use serde_json::Value;

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_orbit/orbit_cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("Orbit fixture readable"))
        .expect("Orbit fixture JSON valid")
}

#[test]
fn match_golden_cases() {
    for case in fixture()["match_cases"]
        .as_array()
        .expect("match_cases is an array")
    {
        let request: OrbitMatchRequest =
            serde_json::from_value(case["request"].clone()).expect("match request valid");
        let expected: OrbitMatchReport =
            serde_json::from_value(case["expected"].clone()).expect("match report valid");

        assert_eq!(match_orbits(&request), expected, "case {}", case["name"]);
    }
}

#[test]
fn parse_golden_cases() {
    for case in fixture()["parse_cases"]
        .as_array()
        .expect("parse_cases is an array")
    {
        let request: OrbitParseRequest =
            serde_json::from_value(case["request"].clone()).expect("parse request valid");
        let expected: OrbitParseResponse =
            serde_json::from_value(case["expected"].clone()).expect("parse response valid");

        assert_eq!(
            parse_orbit_filenames(&request),
            expected,
            "case {}",
            case["name"]
        );
    }
}
