use insar_core::project_stac_scientific_metadata;
use serde_json::json;

fn properties(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn single_acquisition_sar_is_not_forced_into_an_insar_pair() {
    let p = properties(json!({
        "sar:frequency_band": "C",
        "sar:polarizations": ["VV", "VH"],
        "sar:observation_direction": "right",
        "sat:orbit_state": "descending",
        "product:type": "SLC",
        "sar:center_frequency": 5.405,
        "sar:pixel_spacing_range": 2.3,
        "sar:pixel_spacing_azimuth": 13.9
    }));
    let m = project_stac_scientific_metadata(&p).unwrap();
    assert_eq!(m.sar.frequency_band.as_deref(), Some("C"));
    assert_eq!(m.sar.product_type.as_deref(), Some("SLC"));
    assert_eq!(m.sar.observation_direction.as_deref(), Some("right"));
    assert_eq!(m.sar.orbit_state.as_deref(), Some("descending"));
    assert!(m.insar_pair.is_none());
}

#[test]
fn proposal_pair_metadata_keeps_original_numeric_values_and_acquisition_order() {
    let p = properties(json!({
        "insar:reference_datetime": "2026-01-01T00:00:00Z",
        "insar:secondary_datetime": "2026-01-13T00:00:00Z",
        "insar:temporal_baseline": 12.0,
        "insar:perpendicular_baseline": -37.4
    }));
    let m = project_stac_scientific_metadata(&p).unwrap();
    let pair = m.insar_pair.unwrap();
    assert_eq!(pair.temporal_baseline_source_value, Some(12.0));
    assert_eq!(pair.perpendicular_baseline_source_value, Some(-37.4));
    assert_eq!(
        pair.secondary_datetime_utc
            .signed_duration_since(pair.reference_datetime_utc)
            .num_days(),
        12
    );
}

#[test]
fn rejects_partial_pairs_ambiguous_geometry_or_illegal_numbers() {
    for p in [
        json!({"insar:reference_datetime": "2026-01-01T00:00:00Z"}),
        json!({"insar:reference_datetime": "2026-01-13T00:00:00Z", "insar:secondary_datetime": "2026-01-13T00:00:00Z"}),
        json!({"insar:reference_datetime": "2026-01-01T00:00:00+00:00", "insar:secondary_datetime": "2026-01-13T00:00:00Z"}),
        json!({"sar:polarizations": ["VV", "VV"]}),
        json!({"sar:observation_direction": "ascending"}),
        json!({"sar:pixel_spacing_range": -1}),
        json!({"sar:frequency_band": "Z"}),
        json!({"sar:center_frequency": 0.0}),
        json!({"insar:perpendicular_baseline": 5}),
        json!({"sat:orbit_state": "right"}),
    ] {
        assert!(project_stac_scientific_metadata(&properties(p)).is_err());
    }
}

#[test]
fn no_implicit_displacement_or_deprecated_sar_field_mapping() {
    let p = properties(json!({
        "sar:product_type": "GRD",
        "sar:instrument_mode": "IW",
        "displacement": 2.0,
        "units": "mm"
    }));
    let m = project_stac_scientific_metadata(&p).unwrap();
    assert!(m.sar.product_type.is_none());
    assert!(m.insar_pair.is_none());
}

#[test]
fn reference_role_is_not_an_assumption_about_chronological_order() {
    let p = properties(json!({
        "insar:reference_datetime": "2026-01-13T00:00:00Z",
        "insar:secondary_datetime": "2026-01-01T00:00:00Z",
        "insar:temporal_baseline": -12.0
    }));
    let m = project_stac_scientific_metadata(&p).unwrap();
    let pair = m.insar_pair.unwrap();
    assert_eq!(pair.temporal_baseline_source_value, Some(-12.0));
    assert_eq!(
        pair.secondary_datetime_utc
            .signed_duration_since(pair.reference_datetime_utc)
            .num_days(),
        -12
    );
}

#[test]
fn all_stac_v1_3_2_named_sar_bands_are_recognized() {
    for band in ["P", "L", "S", "C", "X", "Ku", "K", "Ka"] {
        let p = properties(json!({"sar:frequency_band": band}));
        let metadata = project_stac_scientific_metadata(&p).unwrap();
        assert_eq!(metadata.sar.frequency_band.as_deref(), Some(band));
    }
}
