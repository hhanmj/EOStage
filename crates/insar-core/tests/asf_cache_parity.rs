use std::fs;
use std::path::PathBuf;

use insar_core::{
    asf_search_cache_key, decide_asf_cache, AsfCacheDecisionKind, AsfCacheDecisionRequest,
    AsfCacheEntry, AsfSceneMetadata, AsfSearchQuery,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/v3_asf/asf_cache_cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("fixture is readable"))
        .expect("fixture is valid JSON")
}

#[test]
fn cache_key_ignores_limit_and_normalizes_equivalent_filters() {
    let fixture = fixture();
    let broad = &fixture["coverage"][1]["cached"];
    let mut first = query(broad, Some(10));
    first.beam_mode = " iw, ew ".to_string();
    first.relative_orbit = Some("42, 40-41".to_string());
    let mut second = first.clone();
    second.max_results = Some(100);
    second.beam_mode = "EW;IW".to_string();
    second.relative_orbit = Some("42;40-41".to_string());

    assert_eq!(
        asf_search_cache_key(&first).unwrap(),
        asf_search_cache_key(&second).unwrap()
    );
}

#[test]
fn cache_key_uses_canonical_true_aoi_geometry() {
    let fixture = fixture();
    let broad = &fixture["coverage"][1]["cached"];
    let mut first = query(broad, Some(10));
    first.aoi = "display-name-does-not-own-identity".to_string();
    first.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    let mut same_geometry = first.clone();
    same_geometry.aoi = "renamed AOI".to_string();
    let mut changed_geometry = first.clone();
    changed_geometry.aoi_geojson = Some(
        json!({
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [112.0, 30.0], [112.0, 31.0], [110.0, 30.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
    );

    assert_eq!(
        asf_search_cache_key(&first).unwrap(),
        asf_search_cache_key(&same_geometry).unwrap()
    );
    assert_ne!(
        asf_search_cache_key(&first).unwrap(),
        asf_search_cache_key(&changed_geometry).unwrap()
    );
}

fn query(value: &Value, max_results: Option<u64>) -> AsfSearchQuery {
    serde_json::from_value(json!({
        "version": value["version"],
        "dataset": value["dataset"],
        "bbox": value["bbox"],
        "aoi": value["aoi"],
        "aoi_geojson": null,
        "start": value["start"],
        "end": value["end"],
        "product_type": value["product_type"],
        "beam_mode": value["beam_mode"],
        "polarization": value["polarization"],
        "orbit_direction": value["orbit_direction"],
        "relative_orbit": value["relative_orbit"],
        "frame": value["frame"],
        "max_results": max_results
    }))
    .unwrap()
}

fn scene(index: usize, row: &Value) -> AsfSceneMetadata {
    serde_json::from_value(json!({
        "scene_id": format!("scene-{index}"),
        "url": null,
        "platform": "S1A",
        "product_type": "SLC",
        "beam_mode": "IW",
        "polarization": "DV",
        "acquisition_datetime": row["acquisition_datetime"],
        "absolute_orbit": 52000 + index,
        "path": null,
        "frame": null,
        "orbit_direction": null,
        "file_size_remote": null,
        "footprint_bbox": null,
        "footprint_geojson": null
    }))
    .unwrap()
}

fn first_number(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|text| text.split(['-', ',']).next())
        .and_then(|text| text.parse().ok())
}

fn matching_scene(value: &Value) -> AsfSceneMetadata {
    let mut scene = scene(0, &json!({"acquisition_datetime": "2024-02-01T00:00:00Z"}));
    if let Some(value) = value["beam_mode"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        scene.beam_mode = value.split(',').next().unwrap().to_string();
    }
    if let Some(value) = value["polarization"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        scene.polarization = value.split(',').next().unwrap().to_string();
    }
    scene.orbit_direction = value["orbit_direction"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    scene.path = first_number(&value["relative_orbit"]);
    scene.frame = first_number(&value["frame"]);
    scene
}

#[test]
fn coverage_decisions_match_v219_fixture() {
    let fixture = fixture();
    for (index, case) in fixture["coverage"].as_array().unwrap().iter().enumerate() {
        let requested = query(&case["requested"], Some(1));
        let entry = AsfCacheEntry {
            cache_key: format!("coverage-{index}"),
            query: query(&case["cached"], None),
            scenes: vec![matching_scene(&case["requested"])],
            complete: true,
            total_count: Some(1),
        };

        let response = decide_asf_cache(&AsfCacheDecisionRequest {
            query: requested,
            entries: vec![entry],
        })
        .unwrap();

        let expected = if case["expected"] == true {
            AsfCacheDecisionKind::Reuse
        } else {
            AsfCacheDecisionKind::Miss
        };
        assert_eq!(response.kind, expected, "case {}", case["name"]);
    }
}

#[test]
fn finite_and_unlimited_reuse_match_v219_fixture() {
    let fixture = fixture();
    let broad = &fixture["coverage"][1]["cached"];
    for (index, case) in fixture["reuse"].as_array().unwrap().iter().enumerate() {
        let scenes = case["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(scene_index, row)| scene(scene_index, row))
            .collect();
        let requested_limit = case["requested_limit"].as_u64();
        let response = decide_asf_cache(&AsfCacheDecisionRequest {
            query: query(broad, requested_limit),
            entries: vec![AsfCacheEntry {
                cache_key: format!("reuse-{index}"),
                query: query(broad, None),
                scenes,
                complete: case["complete"].as_bool().unwrap(),
                total_count: None,
            }],
        })
        .unwrap();

        let expected = if case["expected_reuse"] == true {
            AsfCacheDecisionKind::Reuse
        } else if requested_limit.is_some() && !case["scenes"].as_array().unwrap().is_empty() {
            AsfCacheDecisionKind::Incremental
        } else {
            AsfCacheDecisionKind::Miss
        };
        assert_eq!(response.kind, expected, "case {}", case["name"]);
        if expected == AsfCacheDecisionKind::Reuse {
            assert_eq!(
                response.scenes.len(),
                case["expected_count"].as_u64().unwrap() as usize
            );
        }
    }
}

#[test]
fn incremental_decision_uses_oldest_cached_acquisition_as_end() {
    let fixture = fixture();
    let case = &fixture["incremental"][0];
    let broad = &fixture["coverage"][1]["cached"];
    let scenes = case["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, row)| scene(index, row))
        .collect();

    let response = decide_asf_cache(&AsfCacheDecisionRequest {
        query: query(broad, Some(4)),
        entries: vec![AsfCacheEntry {
            cache_key: "incremental".to_string(),
            query: query(broad, None),
            scenes,
            complete: false,
            total_count: Some(12),
        }],
    })
    .unwrap();

    assert_eq!(response.kind, AsfCacheDecisionKind::Incremental);
    assert_eq!(response.missing_count, 1);
    assert_eq!(response.incremental_start.as_deref(), Some(""));
    assert_eq!(
        response.incremental_end.as_deref(),
        case["expected_end"].as_str()
    );
}
