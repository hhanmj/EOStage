use insar_core::CatalogSearchQuery;
use insar_provider::asf::nisar::build_nisar_rslc_query;
use insar_provider::asf::query::AsfCmrQuery;
use insar_provider::catalog::{
    CollectionProfile, CollectionProfileRegistry, NISAR_LOGICAL_DATASET,
    NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
};
use serde_json::{json, Map, Value};

fn profile() -> &'static CollectionProfile {
    CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists")
}

fn query(filters: Value) -> CatalogSearchQuery {
    let filters = filters
        .as_object()
        .cloned()
        .expect("test filters are an object");
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[100.0, -7.0], [104.0, -7.0], [104.0, 1.0], [100.0, 1.0], [100.0, -7.0]]]
        },
        "cursor": null,
        "result_limit": 100,
        "filters": filters,
    }))
    .expect("closed NISAR catalogue query")
}

fn param_values<'a>(plan: &'a AsfCmrQuery, name: &str) -> Vec<&'a str> {
    plan.common_params
        .iter()
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .collect()
}

fn attributes(plan: &AsfCmrQuery) -> Vec<&str> {
    param_values(plan, "attribute[]")
}

#[test]
fn exact_evidence_backed_filters_translate_without_global_or_or_fake_s_band() {
    let plan = build_nisar_rslc_query(
        &query(json!({
            "datetime": {
                "start": "2026-06-16T22:50:00Z",
                "end": "2026-06-16T23:30:00Z"
            },
            "production_configuration": "pr",
            "frame_coverage": "partial",
            "frequency_a_polarization": "hh+hv",
            "frequency_b_polarization": "vv",
            "range_bandwidth": "40+5",
            "joint_observation": true,
            "flight_direction": "descending",
            "look_direction": "left",
            "track": 39,
            "relative_orbit": 39,
            "frame": 46
        })),
        profile(),
    )
    .expect("the complete evidence-backed filter set translates");

    assert_eq!(
        param_values(&plan, "collection_concept_id"),
        vec![NISAR_NATIVE_COLLECTION_ID]
    );
    assert!(param_values(&plan, "short_name").is_empty());
    assert!(param_values(&plan, "version").is_empty());
    assert_eq!(
        param_values(&plan, "temporal"),
        vec!["2026-06-16T22:50:00Z,2026-06-16T23:30:00Z"]
    );
    assert_eq!(param_values(&plan, "page_size"), vec!["100"]);
    assert_eq!(
        param_values(&plan, "sort_key[]"),
        vec!["start_date", "granule_ur"]
    );
    assert_eq!(
        param_values(&plan, "options[attribute][exclude_collection]"),
        vec!["true"]
    );
    assert!(param_values(&plan, "options[attribute][or]").is_empty());
    assert_eq!(
        attributes(&plan),
        vec![
            "string,PRODUCT_TYPE,RSLC",
            "string,PRODUCTION_PIPELINE,PR",
            "string,FULL_FRAME,FALSE",
            "string,FREQUENCY_A_POLARIZATION_CONCAT,HH+HV",
            "string,FREQUENCY_B_POLARIZATION_CONCAT,VV",
            "string,RANGE_BANDWIDTH_CONCAT,40+5",
            "string,JOINT_OBSERVATION,TRUE",
            "string,ASCENDING_DESCENDING,DESCENDING",
            "int,TRACK_NUMBER,39",
            "int,FRAME_NUMBER,46",
        ]
    );
    assert!(attributes(&plan)
        .iter()
        .all(|value| !value.contains("LOOK_DIRECTION")));
    assert!(attributes(&plan)
        .iter()
        .all(|value| !value.contains("RELATIVE_ORBIT")));
    assert!(attributes(&plan)
        .iter()
        .all(|value| !value.contains("S_BAND")));
    assert_eq!(
        plan.complete_batch_local_filters,
        vec![("look_direction".to_owned(), "LEFT".to_owned())]
    );
    assert_eq!(plan.result_limit, Some(100));
    assert_eq!(plan.provider_page_size, 100);
    assert_eq!(plan.polygons, vec!["100,-7,104,-7,104,1,100,1,100,-7"]);
    assert!(!plan.is_multi_polygon);
}

#[test]
fn multipolygon_and_track_aliases_are_deterministic_and_conflicts_fail_closed() {
    let mut multi = query(json!({"relative_orbit": 40, "look_direction": "RIGHT"}));
    multi.aoi_geojson = serde_json::from_value(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[100.0, 20.0], [101.0, 20.0], [101.0, 21.0], [100.0, 21.0], [100.0, 20.0]]],
            [[[120.0, 40.0], [121.0, 40.0], [121.0, 41.0], [120.0, 41.0], [120.0, 40.0]]]
        ]
    }))
    .expect("polygon map");
    let plan = build_nisar_rslc_query(&multi, profile()).expect("aliases translate");

    assert!(plan.is_multi_polygon);
    assert_eq!(
        plan.polygons,
        vec![
            "100,20,121,20,121,30.5,100,30.5,100,20",
            "100,30.5,121,30.5,121,41,100,41,100,30.5",
        ]
    );
    assert_eq!(
        attributes(&plan)
            .into_iter()
            .filter(|value| value.contains("TRACK_NUMBER"))
            .collect::<Vec<_>>(),
        vec!["int,TRACK_NUMBER,40"]
    );
    assert_eq!(
        plan.complete_batch_local_filters,
        vec![("look_direction".to_owned(), "RIGHT".to_owned())]
    );

    for filters in [
        json!({"track": 40}),
        json!({"relative_orbit": 40}),
        json!({"track": 40, "relative_orbit": 40}),
    ] {
        let plan = build_nisar_rslc_query(&query(filters), profile()).unwrap();
        assert_eq!(
            attributes(&plan)
                .into_iter()
                .filter(|value| value.contains("TRACK_NUMBER"))
                .collect::<Vec<_>>(),
            vec!["int,TRACK_NUMBER,40"]
        );
    }

    let error = build_nisar_rslc_query(
        &query(json!({"track": 39, "relative_orbit": 40})),
        profile(),
    )
    .unwrap_err();
    assert_eq!(error.code, "CATALOGUE_FILTER_INVALID");
    assert_eq!(error.debug_context["field"], "track");
}

#[test]
fn frequency_filters_accept_a_only_b_only_dual_and_unconstrained_searches() {
    for (filters, expected) in [
        (json!({}), Vec::<&str>::new()),
        (
            json!({"frequency_a_polarization": "HH"}),
            vec!["string,FREQUENCY_A_POLARIZATION_CONCAT,HH"],
        ),
        (
            json!({"frequency_b_polarization": "VV"}),
            vec!["string,FREQUENCY_B_POLARIZATION_CONCAT,VV"],
        ),
        (
            json!({
                "frequency_a_polarization": "HH",
                "frequency_b_polarization": "HH+HV"
            }),
            vec![
                "string,FREQUENCY_A_POLARIZATION_CONCAT,HH",
                "string,FREQUENCY_B_POLARIZATION_CONCAT,HH+HV",
            ],
        ),
        (
            json!({"frequency_a_polarization": "HH+HV+VH+VV"}),
            vec!["string,FREQUENCY_A_POLARIZATION_CONCAT,HH+HV+VH+VV"],
        ),
        (
            json!({
                "frequency_a_polarization": "VV+VH",
                "frequency_b_polarization": "VV+VH"
            }),
            vec![
                "string,FREQUENCY_A_POLARIZATION_CONCAT,VV+VH",
                "string,FREQUENCY_B_POLARIZATION_CONCAT,VV+VH",
            ],
        ),
    ] {
        let plan = build_nisar_rslc_query(&query(filters), profile()).unwrap();
        let frequency_attributes = attributes(&plan)
            .into_iter()
            .filter(|value| value.contains("FREQUENCY_"))
            .collect::<Vec<_>>();
        assert_eq!(frequency_attributes, expected);
        assert!(param_values(&plan, "options[attribute][or]").is_empty());
    }
}

#[test]
fn invalid_types_values_and_unreviewed_keys_are_rejected_before_network_work() {
    let invalid: Vec<(&str, Value)> = vec![
        ("production_configuration", json!("UR")),
        ("frame_coverage", json!("SUBSET")),
        ("frequency_a_polarization", json!("HV")),
        ("frequency_b_polarization", json!("HH+VV")),
        ("range_bandwidth", json!("5&attribute[]=unsafe")),
        ("joint_observation", json!("true")),
        ("flight_direction", json!("NORTH")),
        ("look_direction", json!("NADIR")),
        ("track", json!(0)),
        ("relative_orbit", json!(0)),
        ("frame", json!(0)),
        ("track", json!(i64::from(i32::MAX) + 1)),
        ("frame", json!(i64::from(i32::MAX) + 1)),
        ("frequency_a_polarization", json!(["HH", "HH+HV"])),
    ];
    for (field, value) in invalid {
        let error = build_nisar_rslc_query(
            &query(Value::Object(Map::from_iter([(field.to_owned(), value)]))),
            profile(),
        )
        .unwrap_err();
        assert_eq!(error.code, "CATALOGUE_FILTER_INVALID", "{field}");
        assert_eq!(error.debug_context["field"], field, "{field}");
    }

    for field in [
        "path",
        "processing_level",
        "quality_status",
        "s_band",
        "raw_query",
        "options[attribute][or]",
    ] {
        let error = build_nisar_rslc_query(
            &query(Value::Object(Map::from_iter([(
                field.to_owned(),
                json!("unsafe"),
            )]))),
            profile(),
        )
        .unwrap_err();
        assert_eq!(error.code, "CATALOGUE_FILTER_UNSUPPORTED", "{field}");
    }

    let mut bad_time = query(json!({
        "datetime": {"start": "2026-06-17T00:00:00Z", "end": "2026-06-16T00:00:00Z"}
    }));
    assert_eq!(
        build_nisar_rslc_query(&bad_time, profile())
            .unwrap_err()
            .code,
        "CATALOGUE_FILTER_INVALID"
    );
    bad_time.filters = Map::new();
    // Slice 3(a): an explicit target above the ceiling is clamped, not rejected.
    bad_time.result_limit = Some(10_001);
    assert_eq!(
        build_nisar_rslc_query(&bad_time, profile())
            .unwrap()
            .result_limit,
        Some(10_000)
    );
    bad_time.result_limit = Some(100);
    bad_time.cursor = Some("external-cursor".to_owned());
    assert_eq!(
        build_nisar_rslc_query(&bad_time, profile())
            .unwrap_err()
            .code,
        "CATALOGUE_CURSOR_UNSUPPORTED"
    );
}

#[test]
fn exact_nisar_identity_is_required_and_other_profiles_cannot_translate() {
    let mut wrong = query(json!({}));
    wrong.native_collection_id = "C2850225585-ASF".to_owned();
    assert_eq!(
        build_nisar_rslc_query(&wrong, profile()).unwrap_err().code,
        "ASF_COLLECTION_IDENTITY_MISMATCH"
    );

    let alos2 = CollectionProfileRegistry
        .get("asf.alos-2.palsar-2.scansar.l1.1.v1")
        .expect("ALOS-2 profile exists");
    assert_eq!(
        build_nisar_rslc_query(&query(json!({})), alos2)
            .unwrap_err()
            .code,
        "ASF_COLLECTION_IDENTITY_MISMATCH"
    );
}
