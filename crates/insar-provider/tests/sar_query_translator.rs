use insar_core::{AsfSearchQuery, SarSearchQuery};
use insar_provider::asf::query::{plan_asf_query, plan_sar_query, AoiPlanMode};
use serde_json::json;

#[test]
fn large_authoritative_aoi_remains_exact_instead_of_becoming_a_simplified_shape() {
    let mut ring = (0..1_024)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / 1_024.0;
            vec![111.5 + angle.cos() * 2.0, 27.5 + angle.sin() * 2.0]
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "aoi_geojson": {"type": "Polygon", "coordinates": [ring]},
        "max_results": 300
    })))
    .expect("large authoritative AOI projects");

    assert_eq!(plan.aoi_mode, AoiPlanMode::Exact);
    assert!(plan.params["intersectsWith"].len() > 12_000);
}

#[test]
fn explicit_packaged_catalog_freezes_a_bounded_working_set_before_background_count() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "platforms": ["ALOS"],
        "processing_levels": ["L1.1"],
        "max_results": 20
    })))
    .expect("explicit ALOS query is valid");

    assert!(!plan.should_count);
}

#[test]
fn bounded_interactive_sentinel1_aoi_queries_do_not_require_a_remote_count() {
    for max_results in [1, 30, 300] {
        let plan = plan_sar_query(&query(json!({
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]]
            },
            "max_results": max_results
        })))
        .expect("bounded Sentinel-1 AOI query is valid");

        assert!(
            !plan.should_count,
            "a bounded {max_results}-result interactive query must not block on provider count",
        );
        assert_eq!(plan.requested_limit, max_results);
    }
}

#[test]
fn blank_sentinel1_quantity_is_an_explicit_counted_all_results_plan_not_a_500_default() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "max_results": null
    })))
    .expect("blank Sentinel-1 quantity is valid");

    assert!(plan.requested_was_unbounded);
    assert!(plan.should_count);
    assert_eq!(plan.requested_limit, 10_000);
    assert_ne!(plan.params["maxResults"], "500");
}

#[test]
fn invalid_raw_aoi_with_bbox_is_rejected_instead_of_using_the_bbox() {
    let error = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [10.0, 8.0], [0.0, 10.0], [6.0, 0.0], [0.0, 0.0]]]
        },
        "bbox": {"west": 0.0, "east": 10.0, "south": 0.0, "north": 10.0, "crs": "EPSG:4326"},
        "max_results": 1
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
}

#[test]
fn valid_raw_aoi_remains_the_remote_projection_authority() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]]
        },
        "bbox": {"west": -5.0, "east": 15.0, "south": -5.0, "north": 15.0, "crs": "EPSG:4326"},
        "max_results": 1
    })))
    .expect("valid raw AOI projects");

    assert_eq!(plan.aoi_mode, AoiPlanMode::Exact);
    assert!(plan.params["intersectsWith"].starts_with("POLYGON ((0 0"));
    assert!(!plan.params["intersectsWith"].contains("-5"));
}

#[test]
fn legacy_invalid_raw_aoi_with_bbox_is_rejected_instead_of_using_the_bbox() {
    let error = plan_asf_query(&legacy_query(json!({
        "version": 1,
        "dataset": "SENTINEL-1",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[0.0, 0.0], [10.0, 8.0], [0.0, 10.0], [6.0, 0.0], [0.0, 0.0]]]
        },
        "bbox": {"west": 0.0, "east": 10.0, "south": 0.0, "north": 10.0, "crs": "EPSG:4326"}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF002");
}

fn query(value: serde_json::Value) -> SarSearchQuery {
    serde_json::from_value(value).unwrap()
}

fn legacy_query(value: serde_json::Value) -> AsfSearchQuery {
    serde_json::from_value(value).unwrap()
}

#[test]
fn expands_the_sentinel_sm_umbrella_and_deduplicates_direct_submodes() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "beam_modes": ["SM", "S1", "S6"]
    })))
    .unwrap();

    assert_eq!(plan.params["beamMode"], "S1,S2,S3,S4,S5,S6");
}

#[test]
fn accepts_direct_sentinel_stripmap_submodes() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "beam_modes": ["S1", "S3", "S6"]
    })))
    .unwrap();

    assert_eq!(plan.params["beamMode"], "S1,S3,S6");
}

#[test]
fn expands_sm_in_the_legacy_cached_search_query() {
    let plan = plan_asf_query(&legacy_query(json!({
        "version": 1,
        "dataset": "SENTINEL-1",
        "beam_mode": "SM"
    })))
    .unwrap();

    assert_eq!(plan.params["beamMode"], "S1,S2,S3,S4,S5,S6");
}

#[test]
fn accepts_direct_stripmap_submodes_in_the_legacy_cached_search_query() {
    let plan = plan_asf_query(&legacy_query(json!({
        "version": 1,
        "dataset": "SENTINEL-1",
        "beam_mode": "S4"
    })))
    .unwrap();

    assert_eq!(plan.params["beamMode"], "S4");
}

#[test]
fn translates_sentinel1_detailed_grd_and_multi_value_filters() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "platforms": ["Sentinel-1A", "Sentinel-1C"],
        "instrument": "C-SAR",
        "processing_levels": ["GRD_HD", "GRD_HS"],
        "product_families": ["GRD"],
        "beam_modes": ["IW", "EW"],
        "polarizations": ["VV+VH", "VV"],
        "flight_direction": "ASCENDING",
        "relative_orbit": 42,
        "frame": 100,
        "start": "2024-01-01T00:00:00Z",
        "end": "2024-01-31T23:59:59Z",
        "bbox": {"west": 100.0, "east": 101.0, "south": 30.0, "north": 31.0, "crs": "EPSG:4326"},
        "max_results": 250
    })))
    .unwrap();

    assert_eq!(plan.params["dataset"], "SENTINEL-1");
    assert_eq!(plan.params["platform"], "Sentinel-1A,Sentinel-1C");
    assert_eq!(plan.params["processingLevel"], "GRD_HD,GRD_HS");
    assert_eq!(plan.params["beamMode"], "IW,EW");
    assert_eq!(plan.params["polarization"], "VV+VH,VV");
    assert_eq!(plan.params["relativeOrbit"], "42");
    assert_eq!(plan.params["frame"], "100");
    assert!(plan.params["intersectsWith"].starts_with("POLYGON"));
}

#[test]
fn normalizes_ui_calendar_dates_to_complete_asf_utc_days() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["SLC"],
        "beam_modes": ["WV"],
        "start": "2026-08-16",
        "end": "2026/08/16",
        "max_results": 20
    })))
    .expect("calendar dates from the packaged UI must be valid ASF filters");

    assert_eq!(
        plan.params.get("start").map(String::as_str),
        Some("2026-08-16T00:00:00Z")
    );
    assert_eq!(
        plan.params.get("end").map(String::as_str),
        Some("2026-08-16T23:59:59Z")
    );
}

#[test]
fn translates_one_precise_grd_without_collapsing_to_broad_grd() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["GRD_HD"]
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("processingLevel").map(String::as_str),
        Some("GRD_HD")
    );
}

#[test]
fn translates_official_slc_burst_dataset_and_exact_ids() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SLC-BURST",
        "platforms": ["Sentinel-1A"],
        "processing_levels": ["BURST"],
        "product_families": ["BURST"],
        "beam_modes": ["IW"],
        "polarizations": ["VV"],
        "burst": {
            "full_burst_id": "048_101784_IW2",
            "relative_burst_id": 101784,
            "absolute_burst_id": 111519098
        }
    })))
    .unwrap();

    assert_eq!(plan.params["dataset"], "SLC-BURST");
    assert_eq!(plan.params["processingLevel"], "BURST");
    assert_eq!(plan.params["fullBurstID"], "048_101784_IW2");
    assert_eq!(plan.params["relativeBurstID"], "101784");
    assert_eq!(plan.params["absoluteBurstID"], "111519098");
}

#[test]
fn rejects_result_only_burst_fields_as_remote_filters() {
    let error = plan_sar_query(&query(json!({
        "dataset": "SLC-BURST",
        "burst": {"subswath": "IW2", "burst_index": 7}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("burst")
    );
}

#[test]
fn preserves_padded_seasat_asf_frame() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SEASAT",
        "asf_frame": "0217"
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("asfframe").map(String::as_str),
        Some("0217")
    );
    assert!(!plan.params.contains_key("frame"));
}

#[test]
fn validates_and_translates_supported_look_direction() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "look_direction": "right"
    })))
    .unwrap();
    assert_eq!(plan.params["lookDirection"], "RIGHT");

    let error = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "look_direction": "SIDEWAYS"
    })))
    .unwrap_err();
    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("look_direction")
    );
}

#[test]
fn keeps_legacy_numeric_frame_requests_compatible() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "frame": 100
    })))
    .unwrap();

    assert_eq!(plan.params.get("frame").map(String::as_str), Some("100"));
}

#[test]
fn translates_sentinel_track_range_without_dropping_the_end() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "track_range": {"start": "10", "end": "20"}
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("relativeOrbit").map(String::as_str),
        Some("10-20")
    );
}

#[test]
fn complete_sentinel_track_range_overrides_conflicting_legacy_scalars() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "relative_orbit": 99,
        "path": 98,
        "track_range": {"start": "10", "end": "20"}
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("relativeOrbit").map(String::as_str),
        Some("10-20")
    );
}

#[test]
fn translates_sentinel_frame_range_without_dropping_the_end() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "frame_range": {"start": "1", "end": "3"}
    })))
    .unwrap();

    assert_eq!(plan.params.get("frame").map(String::as_str), Some("1-3"));
}

#[test]
fn complete_sentinel_frame_range_overrides_conflicting_legacy_scalars() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "frame": 99,
        "asf_frame": "0100",
        "frame_range": {"start": "1", "end": "3"}
    })))
    .unwrap();

    assert_eq!(plan.params.get("frame").map(String::as_str), Some("1-3"));
}

#[test]
fn complete_alos_track_range_ignores_stale_relative_orbit() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "relative_orbit": 99,
        "track_range": {"start": "10", "end": "20"}
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("relativeOrbit").map(String::as_str),
        Some("10-20")
    );
}

#[test]
fn translates_alos_padded_asf_frame_range() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "frame_range": {"start": "001", "end": "010"}
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("asfframe").map(String::as_str),
        Some("001-010")
    );
    assert!(!plan.params.contains_key("frame"));
}

#[test]
fn preserves_seasat_leading_zeros_across_the_complete_range() {
    let plan = plan_sar_query(&query(json!({
        "dataset": "SEASAT",
        "frame_range": {"start": "0208", "end": "0210"}
    })))
    .unwrap();

    assert_eq!(
        plan.params.get("asfframe").map(String::as_str),
        Some("0208-0210")
    );
}

#[test]
fn rejects_official_mission_filter_values_outside_their_ranges() {
    let cases = [
        (
            json!({"dataset": "SENTINEL-1", "relative_orbit": 176}),
            "relative_orbit",
        ),
        (json!({"dataset": "SENTINEL-1", "frame": 7201}), "frame"),
        (
            json!({"dataset": "ALOS PALSAR", "path": 672}),
            "relative_orbit",
        ),
        (
            json!({"dataset": "ALOS PALSAR", "asf_frame": "7201"}),
            "frame",
        ),
        (json!({"dataset": "SEASAT", "path": 244}), "relative_orbit"),
        (json!({"dataset": "SEASAT", "asf_frame": "0207"}), "frame"),
        (json!({"dataset": "SEASAT", "asf_frame": "3459"}), "frame"),
        (json!({"dataset": "SEASAT", "asf_frame": "208"}), "frame"),
    ];

    for (payload, field) in cases {
        let error = plan_sar_query(&query(payload.clone())).unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_INVALID", "{payload}");
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some(field),
            "{payload}"
        );
    }

    let s1_orbit_err = plan_sar_query(&query(
        json!({"dataset": "SENTINEL-1", "relative_orbit": 999}),
    ))
    .unwrap_err();
    assert_eq!(
        s1_orbit_err
            .debug_context
            .get("reason")
            .and_then(|v| v.as_str()),
        Some("out_of_range")
    );
    assert_eq!(
        s1_orbit_err
            .debug_context
            .get("min")
            .and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        s1_orbit_err
            .debug_context
            .get("max")
            .and_then(|v| v.as_u64()),
        Some(175)
    );
}

#[test]
fn rejects_asf_removed_legacy_missions_with_the_typed_reason() {
    // ASF removed ERS, JERS-1 and RADARSAT-1 from its searchable interfaces
    // (Owner evidence 2026-10-31): their queries are rejected up front with
    // the typed removal reason instead of the generic unsupported-dataset
    // copy, before any filter vocabulary is consulted.
    for payload in [
        json!({"dataset": "ERS", "platforms": ["ERS-1"], "path": 2411}),
        json!({"dataset": "JERS-1", "path": 659}),
        json!({"dataset": "RADARSAT-1", "asf_frame": "901"}),
        json!({"dataset": "radarsat-1"}),
    ] {
        let error = plan_sar_query(&query(payload.clone())).unwrap_err();
        assert_eq!(error.code, "ASF_DATASET_REMOVED", "{payload}");
        assert_eq!(
            error.user_message, "ASF 已从检索接口下线该数据集",
            "{payload}"
        );
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some("dataset"),
            "{payload}"
        );
    }

    // An unknown dataset keeps the generic unsupported copy.
    let error = plan_sar_query(&query(json!({"dataset": "NOT-A-MISSION"}))).unwrap_err();
    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(error.user_message, "ASF Provider 不支持该数据集");
}

#[test]
fn accepts_official_mission_filter_boundaries() {
    let cases = [
        json!({"dataset": "SENTINEL-1", "track_range": {"start": "1", "end": "175"}, "frame_range": {"start": "0", "end": "7200"}}),
        json!({"dataset": "ALOS PALSAR", "track_range": {"start": "1", "end": "671"}, "frame_range": {"start": "0", "end": "7200"}}),
        json!({"dataset": "SEASAT", "track_range": {"start": "1", "end": "243"}, "frame_range": {"start": "0208", "end": "3458"}}),
    ];

    for payload in cases {
        plan_sar_query(&query(payload.clone())).unwrap_or_else(|error| {
            panic!("expected official boundary to be accepted for {payload}: {error:?}")
        });
    }
}

#[test]
fn retired_ers_queries_never_reach_the_track_range_vocabulary() {
    // ERS is retired (ASF removed it from searchable interfaces,
    // Owner evidence 2026-10-31): even the previously safe single-platform
    // range is now rejected before the track-range vocabulary runs.
    for payload in [
        json!({"dataset": "ERS", "path": 501}),
        json!({"dataset": "ERS", "platforms": ["ERS-1", "ERS-2"], "path": 501}),
        json!({"dataset": "ERS", "platforms": ["ERS-1"], "path": 2410}),
    ] {
        let error = plan_sar_query(&query(payload.clone())).unwrap_err();
        assert_eq!(error.code, "ASF_DATASET_REMOVED", "{payload}");
    }
}

#[test]
fn legacy_cached_search_cannot_bypass_sentinel_path_or_frame_ranges() {
    for payload in [
        json!({"version": 1, "dataset": "SENTINEL-1", "relative_orbit": "176"}),
        json!({"version": 1, "dataset": "SENTINEL-1", "frame": "7201"}),
        json!({"version": 1, "dataset": "SENTINEL-1", "relative_orbit": "1-176"}),
    ] {
        let error = plan_asf_query(&legacy_query(payload.clone())).unwrap_err();
        assert_eq!(error.code, "ASF002", "{payload}");
    }
}

#[test]
fn rejects_a_range_for_an_unsupported_dataset_with_a_typed_error() {
    let error = plan_sar_query(&query(json!({
        "dataset": "UNSUPPORTED-DATASET",
        "frame_range": {"start": "1", "end": "3"}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("dataset")
    );
}

#[test]
fn rejects_a_range_with_an_invalid_end_instead_of_using_its_start() {
    let error = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "relative_orbit": 10,
        "track_range": {"start": "10", "end": "twenty"}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("track_range")
    );
}

#[test]
fn rejects_an_incomplete_range_instead_of_falling_back_to_a_scalar() {
    let error = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "relative_orbit": 10,
        "track_range": {"start": "10"}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("track_range")
    );
}

#[test]
fn rejects_a_padded_endpoint_that_violates_the_descriptor_pattern() {
    let error = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "frame_range": {"start": "001", "end": "01A"}
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("frame_range")
    );
}

#[test]
fn rejects_descending_integer_and_padded_ranges() {
    for (dataset, field, start, end) in [
        ("SENTINEL-1", "track_range", "20", "10"),
        ("ALOS PALSAR", "frame_range", "010", "002"),
    ] {
        let error = plan_sar_query(&query(json!({
            "dataset": dataset,
            (field): {"start": start, "end": end}
        })))
        .unwrap_err();

        assert_eq!(error.code, "ASF_QUERY_INVALID");
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some(field)
        );
        assert_eq!(error.user_message, "筛选范围起始值不能大于结束值");
    }
}

#[test]
fn validates_sentinel_detailed_grd_polarization_cardinality() {
    for level in ["GRD_HD", "GRD_MD"] {
        let error = plan_sar_query(&query(json!({
            "dataset": "SENTINEL-1",
            "processing_levels": [level],
            "polarizations": ["VV"]
        })))
        .unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_INVALID", "{level}");
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some("polarizations"),
            "{level}"
        );
    }

    for level in ["GRD_HS", "GRD_MS"] {
        let error = plan_sar_query(&query(json!({
            "dataset": "SENTINEL-1",
            "processing_levels": [level],
            "polarizations": ["VV+VH"]
        })))
        .unwrap_err();
        assert_eq!(error.code, "ASF_QUERY_INVALID", "{level}");
        assert_eq!(
            error
                .debug_context
                .get("field")
                .and_then(|value| value.as_str()),
            Some("polarizations"),
            "{level}"
        );
    }
}

#[test]
fn new_sentinel_catalog_queries_reject_historical_grd_fd() {
    let error = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "processing_levels": ["GRD_FD"]
    })))
    .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(
        error
            .debug_context
            .get("field")
            .and_then(|value| value.as_str()),
        Some("processing_levels")
    );
}

#[test]
fn accepts_matching_or_unspecified_sentinel_grd_polarization() {
    for value in [
        json!({
            "dataset": "SENTINEL-1",
            "processing_levels": ["GRD_HD"],
            "polarizations": ["VV+VH"]
        }),
        json!({
            "dataset": "SENTINEL-1",
            "processing_levels": ["GRD_HS"],
            "polarizations": ["VV"]
        }),
        json!({
            "dataset": "SENTINEL-1",
            "processing_levels": ["GRD_HD"]
        }),
    ] {
        plan_sar_query(&query(value)).unwrap();
    }
}

#[test]
fn translates_alos_without_sentinel_only_parameters() {
    let alos = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "platforms": ["ALOS"],
        "instrument": "PALSAR",
        "processing_levels": ["L1.1"],
        "beam_modes": ["FBS"],
        "polarizations": ["HH"],
        "look_direction": "RIGHT",
        "path": 123
    })))
    .unwrap();

    assert_eq!(alos.params["dataset"], "ALOS PALSAR");
    assert_eq!(alos.params["processingLevel"], "L1.1");
    assert_eq!(alos.params["relativeOrbit"], "123");
    assert_eq!(alos.params["lookDirection"], "RIGHT");
}

#[test]
fn retired_missions_are_rejected_before_vocabulary_translation() {
    // ERS and JERS-1 are retired (ASF removed them from searchable
    // interfaces, Owner evidence 2026-10-31): any query for them is
    // rejected with the typed removal reason before translation.
    for dataset in ["ERS", "JERS-1"] {
        let error = plan_sar_query(&query(json!({ "dataset": dataset }))).unwrap_err();
        assert_eq!(error.code, "ASF_DATASET_REMOVED", "{dataset}");

        let error = plan_sar_query(&query(json!({
            "dataset": dataset,
            "beam_modes": ["STD"]
        })))
        .unwrap_err();
        assert_eq!(error.code, "ASF_DATASET_REMOVED", "{dataset}");
    }
}

#[test]
fn rejects_cross_mission_and_burst_combinations() {
    let invalid = plan_sar_query(&query(json!({
        "dataset": "ALOS PALSAR",
        "processing_levels": ["GRD_HD"],
        "beam_modes": ["IW"]
    })))
    .unwrap_err();
    let burst = plan_sar_query(&query(json!({
        "dataset": "SENTINEL-1",
        "burst": {"subswath": "IW1", "burst_index": 2}
    })))
    .unwrap_err();

    assert_eq!(invalid.code, "ASF_QUERY_UNSUPPORTED");
    assert_eq!(burst.code, "ASF_QUERY_UNSUPPORTED");
}
