use std::fs;
use std::path::PathBuf;

use insar_core::{parse_sentinel1_identifier, sentinel1_relative_orbit};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    valid: Vec<ValidCase>,
    invalid: Vec<String>,
}

#[derive(Deserialize)]
struct ValidCase {
    input: String,
    expected: Value,
    derived_relative_orbit: Option<u16>,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_pure_logic/sentinel1.json");
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn sentinel1_parser_matches_python_fixture_field_for_field() {
    for case in fixture().valid {
        let parsed = parse_sentinel1_identifier(&case.input).unwrap();
        let serialized = serde_json::to_value(&parsed).unwrap();

        assert_eq!(serialized, case.expected, "input: {}", case.input);
        assert_eq!(
            sentinel1_relative_orbit(&parsed.platform, Some(parsed.absolute_orbit)),
            case.derived_relative_orbit,
            "input: {}",
            case.input,
        );
        assert!(serialized.get("flight_direction").is_none());
        assert!(serialized.get("frame").is_none());
    }
}

#[test]
fn sentinel1_parser_rejects_invalid_rows_with_safe_asf001_error() {
    for input in fixture().invalid {
        let error = parse_sentinel1_identifier(&input).unwrap_err();

        assert_eq!(error.code, "ASF001");
        assert_eq!(error.user_message, "无法识别 Sentinel-1 影像名称");
        if !input.is_empty() {
            assert!(!error.to_string().contains(&input));
        }
    }
}

#[test]
fn sentinel1_relative_orbit_does_not_guess_for_new_platforms() {
    assert_eq!(sentinel1_relative_orbit("S1C", Some(123)), None);
    assert_eq!(sentinel1_relative_orbit("S1D", Some(456)), None);
    assert_eq!(sentinel1_relative_orbit("S1A", None), None);
}
