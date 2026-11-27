use std::fs;
use std::path::PathBuf;

use insar_core::polygonal_topology::{
    MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES, MAX_POLYGONAL_COMPONENTS,
    MAX_POLYGONAL_RINGS_PER_POLYGON,
};
use insar_core::{
    normalize_asf_features, validated_polygonal_geojson, AsfNormalizeRequest, AsfSceneMetadata,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/v3_asf/asf_normalization_cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("fixture is readable"))
        .expect("fixture is valid JSON")
}

fn request(case: &Value) -> AsfNormalizeRequest {
    let query = &case["query"];
    serde_json::from_value(json!({
        "query": {
            "version": 1,
            "dataset": "SENTINEL-1",
            "bbox": null,
            "aoi": "fixture-aoi",
            "aoi_geojson": query["aoi_geojson"],
            "start": "",
            "end": "",
            "product_type": "",
            "beam_mode": query["beam_mode"],
            "polarization": query["polarization"],
            "orbit_direction": query["orbit_direction"],
            "relative_orbit": null,
            "frame": null,
            "max_results": query["max_results"]
        },
        "features": case["features"]
    }))
    .expect("fixture request matches generated contract")
}

fn custom_request(aoi_geojson: Value, features: Vec<Value>) -> AsfNormalizeRequest {
    serde_json::from_value(json!({
        "query": {
            "version": 1,
            "dataset": "SENTINEL-1",
            "bbox": null,
            "aoi": "longitude-domain-test",
            "aoi_geojson": aoi_geojson,
            "start": "",
            "end": "",
            "product_type": "SLC",
            "beam_mode": "IW",
            "polarization": "DV",
            "orbit_direction": "",
            "relative_orbit": null,
            "frame": null,
            "max_results": null
        },
        "features": features
    }))
    .unwrap()
}

fn polygon(west: f64, south: f64, east: f64, north: f64) -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, south], [east, south], [east, north],
            [west, north], [west, south]
        ]]
    })
}

fn feature(scene_id: &str, geometry: Value) -> Value {
    json!({
        "type": "Feature",
        "properties": {"sceneName": scene_id},
        "geometry": geometry
    })
}

fn scene_ids(response: &insar_core::AsfNormalizeResponse) -> Vec<&str> {
    response
        .scenes
        .iter()
        .map(|scene| scene.scene_id.as_str())
        .collect()
}

fn new_zealand_geometry() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[166,-47],[179,-47],[179,-34],[166,-34],[166,-47]]],
            [[[-179,-45],[-175,-45],[-175,-35],[-179,-35],[-179,-45]]]
        ]
    })
}

fn full_world_component_union() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[170,0],[-170,0],[-170,1],[170,1],[170,0]]],
            [[[-170,2],[0,2],[0,3],[-170,3],[-170,2]]],
            [[[0,4],[170,4],[170,5],[0,5],[0,4]]]
        ]
    })
}

fn periodic_overlap_geometry() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[170,0],[-170,0],[-170,10],[170,10],[170,0]]],
            [[[171,1],[179,1],[179,9],[171,9],[171,1]]]
        ]
    })
}

fn projection_self_cross_geometry() -> Value {
    json!({
        "type": "Polygon",
        "coordinates": [[[-120,1],[-60,2],[170,0],[-120,4],[-120,1]]]
    })
}

fn dense_crossing_rectangle(vertical_steps: usize) -> Value {
    let mut ring = vec![json!([170.0, 0.0]), json!([-170.0, 0.0])];
    ring.extend(
        (1..=vertical_steps)
            .map(|index| json!([-170.0, 10.0 * index as f64 / vertical_steps as f64])),
    );
    ring.push(json!([170.0, 10.0]));
    ring.extend(
        (0..vertical_steps)
            .rev()
            .map(|index| json!([170.0, 10.0 * index as f64 / vertical_steps as f64])),
    );
    json!({"type": "Polygon", "coordinates": [ring]})
}

fn rectangular_ring(west: f64, south: f64, east: f64, north: f64) -> Vec<(f64, f64)> {
    vec![
        (west, south),
        (east, south),
        (east, north),
        (west, north),
        (west, south),
    ]
}

fn dense_rectangular_ring(
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    bottom_vertices: usize,
) -> Vec<(f64, f64)> {
    let mut coordinates = (0..bottom_vertices)
        .map(|index| {
            let fraction = index as f64 / (bottom_vertices - 1) as f64;
            (west + (east - west) * fraction, south)
        })
        .collect::<Vec<_>>();
    coordinates.extend([(east, north), (west, north), (west, south)]);
    coordinates
}

fn hole_rich_disjoint_components() -> Value {
    json!({
        "type": "MultiPolygon",
        "coordinates": [
            [
                rectangular_ring(0.0, 0.0, 10.0, 10.0),
                dense_rectangular_ring(1.0, 1.0, 9.0, 9.0, 1_980)
            ],
            [
                rectangular_ring(2.0, 2.0, 8.0, 8.0),
                dense_rectangular_ring(3.0, 3.0, 7.0, 7.0, 1_980)
            ]
        ]
    })
}

const NORMAL: &str = "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_A001";
const CROSSING: &str = "S1A_IW_SLC__1SDV_20240102T100000_20240102T100027_052001_064ABD_A002";
const EDGE_TOUCH: &str = "S1A_IW_SLC__1SDV_20240103T100000_20240103T100027_052002_064ABE_A003";
const POINT_TOUCH: &str = "S1A_IW_SLC__1SDV_20240104T100000_20240104T100027_052003_064ABF_A004";

#[test]
fn normalizer_equal_aligned_overlap_preserves_first_provider_occurrence() {
    let response = normalize_asf_features(&custom_request(
        new_zealand_geometry(),
        vec![
            feature(NORMAL, polygon(166.0, -45.0, 168.0, -35.0)),
            feature(CROSSING, polygon(178.0, -45.0, -178.0, -35.0)),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [NORMAL, CROSSING]);
}

#[test]
fn normalizer_higher_aligned_coverage_overrides_source_order() {
    let response = normalize_asf_features(&custom_request(
        new_zealand_geometry(),
        vec![
            feature(NORMAL, polygon(166.0, -45.0, 168.0, -35.0)),
            feature(CROSSING, polygon(160.0, -45.0, 170.0, -35.0)),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_retains_closed_interval_edge_and_point_touches_at_zero_coverage() {
    let response = normalize_asf_features(&custom_request(
        polygon(0.0, 0.0, 10.0, 10.0),
        vec![
            feature(EDGE_TOUCH, polygon(10.0, 2.0, 20.0, 8.0)),
            feature(POINT_TOUCH, polygon(10.0, 10.0, 20.0, 20.0)),
            feature(NORMAL, polygon(0.0, 0.0, 1.0, 1.0)),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [NORMAL, EDGE_TOUCH, POINT_TOUCH]);
}

#[test]
fn normalizer_retains_both_antimeridian_seam_aliases_that_touch_the_aoi_boundary() {
    for (aoi, footprint) in [
        (
            polygon(170.0, 0.0, 180.0, 10.0),
            polygon(-180.0, 2.0, -170.0, 8.0),
        ),
        (
            polygon(-180.0, 0.0, -170.0, 10.0),
            polygon(170.0, 2.0, 180.0, 8.0),
        ),
    ] {
        let response =
            normalize_asf_features(&custom_request(aoi, vec![feature(EDGE_TOUCH, footprint)]))
                .unwrap();

        assert_eq!(scene_ids(&response), [EDGE_TOUCH]);
    }
}

#[test]
fn normalizer_retains_decimal_seam_touch_in_the_projected_f64_domain() {
    let response = normalize_asf_features(&custom_request(
        polygon(179.95, 0.0, -0.1, 10.0),
        vec![feature(EDGE_TOUCH, polygon(-0.1, 2.0, 0.0, 8.0))],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [EDGE_TOUCH]);
}

#[test]
fn normalizer_sums_both_periodic_footprint_copies_for_global_coverage() {
    let response = normalize_asf_features(&custom_request(
        polygon(-180.0, 0.0, 180.0, 10.0),
        vec![
            feature(NORMAL, polygon(-180.0, 0.0, -165.0, 10.0)),
            feature(CROSSING, polygon(170.0, 0.0, -170.0, 10.0)),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_ranks_a_full_world_footprint_across_both_sides_of_a_seam_aoi() {
    let response = normalize_asf_features(&custom_request(
        polygon(170.0, 0.0, -170.0, 10.0),
        vec![
            feature(NORMAL, polygon(170.0, 0.0, 180.0, 10.0)),
            feature(CROSSING, polygon(-180.0, 0.0, 180.0, 10.0)),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_keeps_exact_results_when_no_bounded_envelope_can_be_proved() {
    let response = normalize_asf_features(&custom_request(
        json!({
            "type": "Polygon",
            "coordinates": [[[0,0],[120,0],[-120,1],[0,0]]]
        }),
        vec![
            feature(CROSSING, polygon(40.0, 0.0, 50.0, 1.0)),
            feature(NORMAL, polygon(0.0, 0.0, 10.0, 1.0)),
        ],
    ))
    .expect("valid exact SearchAPI AOI must not require a derived bbox envelope");

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_filters_full_world_component_unions_outside_aoi_latitude() {
    let response = normalize_asf_features(&custom_request(
        polygon(-180.0, 10.0, 180.0, 11.0),
        vec![feature(CROSSING, full_world_component_union())],
    ))
    .unwrap();

    assert!(response.scenes.is_empty());
}

#[test]
fn normalizer_preserves_provider_order_when_aoi_components_overlap_after_projection() {
    let response = normalize_asf_features(&custom_request(
        periodic_overlap_geometry(),
        vec![
            feature(CROSSING, polygon(0.0, 0.0, 1.0, 1.0)),
            feature(NORMAL, polygon(172.0, 2.0, 173.0, 3.0)),
        ],
    ))
    .expect("valid exact AOI rows survive ambiguous derived topology");

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_preserves_footprint_when_its_components_overlap_after_projection() {
    let response = normalize_asf_features(&custom_request(
        polygon(170.0, 20.0, -170.0, 30.0),
        vec![feature(CROSSING, periodic_overlap_geometry())],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [CROSSING]);
}

#[test]
fn normalizer_preserves_all_provider_order_when_one_projected_footprint_is_ambiguous() {
    let response = normalize_asf_features(&custom_request(
        polygon(170.0, 0.0, -170.0, 10.0),
        vec![
            feature(NORMAL, polygon(170.0, 0.0, 171.0, 10.0)),
            feature(CROSSING, periodic_overlap_geometry()),
        ],
    ))
    .unwrap();

    assert_eq!(scene_ids(&response), [NORMAL, CROSSING]);
}

#[test]
fn normalizer_preserves_exact_rows_when_relate_pair_work_exhausts_the_budget() {
    let footprint = hole_rich_disjoint_components();
    assert!(validated_polygonal_geojson(footprint.as_object().unwrap()).is_some());

    let response = normalize_asf_features(&custom_request(
        polygon(0.0, 0.0, 10.0, 10.0),
        vec![
            feature(NORMAL, polygon(0.0, 0.0, 1.0, 1.0)),
            feature(CROSSING, footprint),
        ],
    ))
    .expect("derived Relate budget exhaustion must preserve exact rows");

    assert_eq!(scene_ids(&response), [NORMAL, CROSSING]);
}

#[test]
fn normalizer_preserves_provider_order_when_one_aoi_component_self_crosses_after_unwrap() {
    let response = normalize_asf_features(&custom_request(
        projection_self_cross_geometry(),
        vec![
            feature(CROSSING, polygon(0.0, 0.0, 1.0, 1.0)),
            feature(NORMAL, polygon(-130.0, 2.0, -125.0, 2.5)),
        ],
    ))
    .expect("valid exact AOI rows survive ambiguous derived topology");

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_keeps_exact_rows_when_near_limit_projected_ring_work_is_ambiguous() {
    let mut aoi = dense_crossing_rectangle(400);
    aoi.as_object_mut().unwrap().insert(
        "authority_padding".to_string(),
        Value::String("x".repeat(MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES * 9 / 10)),
    );
    let serialized_size = serde_json::to_vec(&aoi).unwrap().len();
    assert!(serialized_size >= MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES * 9 / 10);
    assert!(serialized_size < MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES);

    let response = normalize_asf_features(&custom_request(
        aoi,
        vec![
            feature(CROSSING, polygon(0.0, 0.0, 1.0, 1.0)),
            feature(NORMAL, polygon(171.0, 1.0, 172.0, 2.0)),
        ],
    ))
    .expect("exact authority survives derived work exhaustion");

    assert_eq!(scene_ids(&response), [CROSSING, NORMAL]);
}

#[test]
fn normalizer_aligns_one_window_seams_and_rejects_greenwich_false_world_copy() {
    let cases = [
        (
            polygon(170.0, 0.0, -180.0, 10.0),
            polygon(172.0, 0.0, 178.0, 10.0),
            polygon(0.0, 0.0, 1.0, 10.0),
        ),
        (
            polygon(-170.0, 0.0, 180.0, 10.0),
            polygon(-178.0, 0.0, -172.0, 10.0),
            polygon(0.0, 0.0, 1.0, 10.0),
        ),
        (
            polygon(-10.0, 0.0, 10.0, 10.0),
            polygon(0.0, 0.0, 2.0, 10.0),
            polygon(175.0, 0.0, -175.0, 10.0),
        ),
    ];

    for (aoi, local, false_world_copy) in cases {
        let response = normalize_asf_features(&custom_request(
            aoi,
            vec![feature(CROSSING, false_world_copy), feature(NORMAL, local)],
        ))
        .unwrap();

        assert_eq!(scene_ids(&response), [NORMAL]);
    }
}

#[test]
fn normalizes_v219_fixture_with_exact_topology_and_authoritative_metadata() {
    let fixture = fixture();
    for case in fixture["cases"].as_array().expect("cases are an array") {
        let expected: Vec<AsfSceneMetadata> =
            serde_json::from_value(case["expected_scenes"].clone()).unwrap();

        let actual = normalize_asf_features(&request(case)).expect("normalization succeeds");

        assert_eq!(actual.scenes, expected, "case {}", case["name"]);
        assert_eq!(actual.errors.len(), 1, "case {}", case["name"]);
        assert_eq!(actual.errors[0].index, 5);
        assert_eq!(actual.errors[0].error.code, "ASF001");
    }
}

#[test]
fn does_not_infer_s1b_direction_path_or_frame_when_metadata_is_missing() {
    let request: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1,
            "dataset": "SENTINEL-1",
            "bbox": null,
            "aoi": "",
            "aoi_geojson": null,
            "start": "",
            "end": "",
            "product_type": "SLC",
            "beam_mode": "IW",
            "polarization": "DV",
            "orbit_direction": "",
            "relative_orbit": null,
            "frame": null,
            "max_results": null
        },
        "features": [{
            "type": "Feature",
            "properties": {
                "sceneName": "S1B_IW_SLC__1SDV_20240206T100000_20240206T100027_034175_064BBB_ABCD"
            },
            "geometry": null
        }]
    }))
    .unwrap();

    let response = normalize_asf_features(&request).unwrap();
    let scene = response.scenes.first().unwrap();
    assert_eq!(scene.orbit_direction, None);
    assert_eq!(scene.path, None);
    assert_eq!(scene.frame, None);
}

#[test]
fn keeps_scene_but_reports_non_polygon_remote_geometry() {
    let request: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": null, "start": "", "end": "", "product_type": "SLC",
            "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
            "relative_orbit": null, "frame": null, "max_results": null
        },
        "features": [{
            "type": "Feature",
            "properties": {
                "sceneName": "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234"
            },
            "geometry": {"type": "Point", "coordinates": [110.0, 30.0]}
        }]
    }))
    .unwrap();

    let response = normalize_asf_features(&request).unwrap();

    assert_eq!(response.scenes.len(), 1);
    assert_eq!(response.scenes[0].footprint_bbox, None);
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.errors[0].error.code, "ASF_GEOMETRY_INVALID");
}

#[test]
fn feature_collection_aoi_fails_closed_before_boolean_operations() {
    let request: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "two-islands",
            "aoi_geojson": {"type": "FeatureCollection", "features": [
                {"type": "Feature", "properties": {}, "geometry": {
                    "type": "Polygon", "coordinates": [[[0,0],[1,0],[1,1],[0,1],[0,0]]]
                }},
                {"type": "Feature", "properties": {}, "geometry": {
                    "type": "Polygon", "coordinates": [[[3,3],[4,3],[4,4],[3,4],[3,3]]]
                }}
            ]},
            "start": "", "end": "", "product_type": "SLC", "beam_mode": "IW",
            "polarization": "DV", "orbit_direction": "", "relative_orbit": null,
            "frame": null, "max_results": null
        },
        "features": [
            {"type": "Feature", "properties": {"sceneName":
                "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234"},
             "geometry": {"type": "Polygon", "coordinates": [[[0.2,0.2],[0.8,0.2],[0.8,0.8],[0.2,0.8],[0.2,0.2]]]}},
            {"type": "Feature", "properties": {"sceneName":
                "S1A_IW_SLC__1SDV_20240113T100000_20240113T100027_052175_064DEF_5678"},
             "geometry": {"type": "Polygon", "coordinates": [[[1.5,1.5],[2,1.5],[2,2],[1.5,2],[1.5,1.5]]]}}
        ]
    }))
    .unwrap();

    let error = normalize_asf_features(&request).unwrap_err();

    assert_eq!(error.code, "AOI001");
}

#[test]
fn invalid_raw_aoi_fails_but_complex_authoritative_aoi_remains_exact_for_local_filtering() {
    let bow_tie: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": {"type": "Polygon", "coordinates": [[[0,0],[10,8],[0,10],[6,0],[0,0]]]},
            "start": "", "end": "", "product_type": "SLC", "beam_mode": "IW",
            "polarization": "DV", "orbit_direction": "", "relative_orbit": null,
            "frame": null, "max_results": null
        },
        "features": []
    }))
    .unwrap();
    assert_eq!(normalize_asf_features(&bow_tie).unwrap_err().code, "AOI001");

    let polygons = (0..(MAX_POLYGONAL_COMPONENTS + 1))
        .map(|index| {
            let west = index as f64 * 2.0;
            vec![vec![
                vec![west, 0.0],
                vec![west + 1.0, 0.0],
                vec![west + 1.0, 1.0],
                vec![west, 1.0],
                vec![west, 0.0],
            ]]
        })
        .collect::<Vec<_>>();
    let overbound: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": {"type": "MultiPolygon", "coordinates": polygons},
            "start": "", "end": "", "product_type": "SLC", "beam_mode": "IW",
            "polarization": "DV", "orbit_direction": "", "relative_orbit": null,
            "frame": null, "max_results": null
        },
        "features": []
    }))
    .unwrap();
    assert!(normalize_asf_features(&overbound).is_ok());
}

#[test]
fn bounded_candidate_and_remote_projection_parser_accepts_limits_and_rejects_one_over() {
    let components = |count| {
        (0..count)
            .map(|index| {
                let west = (index as f64) * 2.0;
                vec![vec![
                    vec![west, 0.0],
                    vec![west + 1.0, 0.0],
                    vec![west + 1.0, 1.0],
                    vec![west, 1.0],
                    vec![west, 0.0],
                ]]
            })
            .collect::<Vec<_>>()
    };
    let at_component_limit = json!({
        "type": "MultiPolygon",
        "coordinates": components(MAX_POLYGONAL_COMPONENTS)
    });
    let over_component_limit = json!({
        "type": "MultiPolygon",
        "coordinates": components(MAX_POLYGONAL_COMPONENTS + 1)
    });
    assert!(validated_polygonal_geojson(at_component_limit.as_object().unwrap()).is_some());
    assert!(validated_polygonal_geojson(over_component_limit.as_object().unwrap()).is_none());

    let polygon_with_holes = |hole_count| {
        let mut rings = vec![vec![
            vec![0.0, 0.0],
            vec![100.0, 0.0],
            vec![100.0, 100.0],
            vec![0.0, 100.0],
            vec![0.0, 0.0],
        ]];
        for index in 0..hole_count {
            let west = 2.0 + (index % 10) as f64 * 8.0;
            let south = 2.0 + (index / 10) as f64 * 8.0;
            rings.push(vec![
                vec![west, south],
                vec![west + 1.0, south],
                vec![west + 1.0, south + 1.0],
                vec![west, south + 1.0],
                vec![west, south],
            ]);
        }
        json!({"type": "Polygon", "coordinates": rings})
    };
    let at_ring_limit = polygon_with_holes(MAX_POLYGONAL_RINGS_PER_POLYGON - 1);
    let over_ring_limit = polygon_with_holes(MAX_POLYGONAL_RINGS_PER_POLYGON);
    assert!(validated_polygonal_geojson(at_ring_limit.as_object().unwrap()).is_some());
    assert!(validated_polygonal_geojson(over_ring_limit.as_object().unwrap()).is_none());
}

#[test]
fn invalid_candidate_geometry_stays_visible_without_boolean_overlap() {
    let request: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": {"type": "Polygon", "coordinates": [[[0,0],[10,0],[10,10],[0,10],[0,0]]]},
            "start": "", "end": "", "product_type": "SLC", "beam_mode": "IW",
            "polarization": "DV", "orbit_direction": "", "relative_orbit": null,
            "frame": null, "max_results": null
        },
        "features": [{
            "type": "Feature",
            "properties": {"sceneName": "S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_1234"},
            "geometry": {"type": "Polygon", "coordinates": [[[0,0],[10,8],[0,10],[6,0],[0,0]]]}
        }]
    }))
    .unwrap();

    let response = normalize_asf_features(&request).unwrap();

    assert_eq!(response.scenes.len(), 1);
    assert_eq!(response.scenes[0].footprint_bbox, None);
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.errors[0].error.code, "ASF_GEOMETRY_INVALID");
}

#[test]
fn sentinel_compatibility_normalizer_preserves_optional_provider_evidence() {
    let request: AsfNormalizeRequest = serde_json::from_value(json!({
        "query": {
            "version": 1, "dataset": "SENTINEL-1", "bbox": null, "aoi": "",
            "aoi_geojson": null, "start": "", "end": "", "product_type": "GRD",
            "beam_mode": "IW", "polarization": "DV", "orbit_direction": "",
            "relative_orbit": null, "frame": null, "max_results": null
        },
        "features": [{
            "type": "Feature",
            "properties": {
                "sceneName": "S1A_IW_GRDH_1SDV_20240101T100000_20240101T100027_052000_064ABC_1234",
                "fileID": "S1-PROVIDER-GRD-HD",
                "fileName": "S1-PROVIDER-GRD-HD.zip",
                "processingLevel": "GRD_HD",
                "stopTime": "2024-01-01T10:00:27Z",
                "browse": ["https://datapool.asf.alaska.edu/BROWSE/S1-PROVIDER-GRD-HD.jpg"],
                "md5sum": "0123456789abcdef0123456789abcdef",
                "centerLat": 30.5,
                "centerLon": 100.5,
                "bytes": 1024
            },
            "geometry": null
        }]
    }))
    .unwrap();

    let response = normalize_asf_features(&request).unwrap();
    let scene = response.scenes.first().unwrap();

    assert_eq!(
        scene.provider_product_id.as_deref(),
        Some("S1-PROVIDER-GRD-HD")
    );
    assert_eq!(scene.file_name.as_deref(), Some("S1-PROVIDER-GRD-HD.zip"));
    assert_eq!(scene.processing_level.as_deref(), Some("GRD_HD"));
    assert_eq!(
        scene.acquisition_stop.as_deref(),
        Some("2024-01-01T10:00:27Z")
    );
    assert_eq!(
        scene.browse_url.as_deref(),
        Some("https://datapool.asf.alaska.edu/BROWSE/S1-PROVIDER-GRD-HD.jpg")
    );
    assert_eq!(scene.checksum_algorithm.as_deref(), Some("md5"));
    assert_eq!(
        scene.checksum_value.as_deref(),
        Some("0123456789abcdef0123456789abcdef")
    );
    assert_eq!(scene.center_latitude, Some(30.5));
    assert_eq!(scene.center_longitude, Some(100.5));
}
