use std::collections::BTreeMap;
use std::sync::Mutex;

use insar_core::polygonal_topology::{MAX_POLYGONAL_COORDS_PER_RING, MAX_POLYGONAL_TOTAL_COORDS};
use insar_core::{ProviderResult, SarRemoteSearchRequest};
use insar_provider::asf::search::{AsfProvider, MetadataTransport};
use serde_json::{json, Value};

struct MetadataFixtureTransport {
    features: Value,
    count: Option<u64>,
    geojson_params: Mutex<Vec<BTreeMap<String, String>>>,
}

impl MetadataTransport for MetadataFixtureTransport {
    fn asf_geojson(
        &self,
        params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        self.geojson_params.lock().unwrap().push(params.clone());
        Ok(json!({"type": "FeatureCollection", "features": self.features}))
    }

    fn asf_count(
        &self,
        _params: &BTreeMap<String, String>,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Option<u64>> {
        Ok(self.count)
    }

    fn cmr_json(
        &self,
        _params: &[(String, String)],
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<Value> {
        panic!("generic ASF coverage search must not route to CMR")
    }
}

fn provider(features: Vec<Value>) -> AsfProvider<MetadataFixtureTransport> {
    let count = features.len() as u64;
    provider_with_count(features, Some(count))
}

fn provider_with_count(
    features: Vec<Value>,
    count: Option<u64>,
) -> AsfProvider<MetadataFixtureTransport> {
    AsfProvider::with_transport(
        MetadataFixtureTransport {
            features: Value::Array(features),
            count,
            geojson_params: Mutex::new(Vec::new()),
        },
        "2026-08-17T00:00:00Z",
    )
}

fn feature(id: &str, geometry: Value) -> Value {
    json!({
        "type": "Feature",
        "properties": {
            "sceneName": format!("S1A_IW_SLC__1SDV_20240101T100000_20240101T100027_052000_064ABC_{id}"),
            "fileID": format!("S1-{id}"),
            "fileName": format!("S1-{id}.zip"),
            "processingLevel": "SLC",
            "platform": "SENTINEL-1A",
            "beamModeType": "IW",
            "polarization": "VV+VH",
            "flightDirection": "ASCENDING"
        },
        "geometry": geometry
    })
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
    let mut ring = vec![(170.0, 0.0), (-170.0, 0.0)];
    ring.extend(
        (1..=vertical_steps).map(|index| (-170.0, 10.0 * index as f64 / vertical_steps as f64)),
    );
    ring.push((170.0, 10.0));
    ring.extend(
        (0..vertical_steps)
            .rev()
            .map(|index| (170.0, 10.0 * index as f64 / vertical_steps as f64)),
    );
    json!({"type": "Polygon", "coordinates": [ring]})
}

fn ring(west: f64, south: f64, east: f64, north: f64) -> Vec<(f64, f64)> {
    vec![
        (west, south),
        (east, south),
        (east, north),
        (west, north),
        (west, south),
    ]
}

fn polygon_with_rings(rings: Vec<Vec<(f64, f64)>>) -> Value {
    json!({"type": "Polygon", "coordinates": rings})
}

fn multipolygon_with_rings(polygons: Vec<Vec<Vec<(f64, f64)>>>) -> Value {
    json!({"type": "MultiPolygon", "coordinates": polygons})
}

fn point(longitude: f64, latitude: f64) -> Value {
    json!({"type": "Point", "coordinates": [longitude, latitude]})
}

fn unmappable_feature() -> Value {
    json!({"type": "Feature", "properties": {}, "geometry": null})
}

fn aoi_query(max_results: u64) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": {
                "type": "Polygon",
                "coordinates": [[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]]
            },
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn antimeridian_aoi_query(max_results: u64) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": {
                "type": "MultiPolygon",
                "coordinates": [
                    [[[166,-47],[179,-47],[179,-34],[166,-34],[166,-47]]],
                    [[[-179,-45],[-175,-45],[-175,-35],[-179,-35],[-179,-45]]]
                ]
            },
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn geometry_aoi_query(aoi_geojson: Value, max_results: u64) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": aoi_geojson,
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn aoi_query_with_geometry_and_bbox(
    aoi_geojson: Value,
    bbox: Value,
    max_results: u64,
) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "aoi_geojson": aoi_geojson,
            "bbox": bbox,
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn bbox_query(max_results: u64) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "bbox": {"west": 0.0, "east": 10.0, "south": 0.0, "north": 10.0, "crs": "EPSG:4326"},
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn standard_bbox() -> Value {
    json!({"west": 0.0, "east": 10.0, "south": 0.0, "north": 10.0, "crs": "EPSG:4326"})
}

fn dense_ring(
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
    multipolygon_with_rings(vec![
        vec![
            ring(0.0, 0.0, 10.0, 10.0),
            dense_ring(1.0, 1.0, 9.0, 9.0, 1_980),
        ],
        vec![
            ring(2.0, 2.0, 8.0, 8.0),
            dense_ring(3.0, 3.0, 7.0, 7.0, 1_980),
        ],
    ])
}

fn query_without_aoi(max_results: u64) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "max_results": max_results
        }
    }))
    .unwrap()
}

fn file_names(response: &insar_core::SarRemoteSearchResponse) -> Vec<Option<&str>> {
    response
        .products
        .iter()
        .map(|product| product.file_name.as_deref())
        .collect()
}

#[test]
fn bounded_aoi_search_selects_highest_coverage_before_truncation() {
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("MISSING", Value::Null),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
        feature("MEDIUM", polygon(0.0, 0.0, 6.0, 10.0)),
    ]);

    let response = provider.search_sar(&aoi_query(2), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-HIGH.zip"), Some("S1-MEDIUM.zip")]
    );
    assert_eq!(
        provider.transport().geojson_params.lock().unwrap()[0]["maxResults"],
        "52"
    );
    assert_eq!(
        response.stats.matched_count, None,
        "bounded numeric mode must not invent a remote total without a count request"
    );
    assert_eq!(response.stats.returned_count, 2);
}

#[test]
fn equal_coverage_polygons_remain_in_provider_order() {
    let provider = provider(vec![
        feature("Z", polygon(0.0, 0.0, 10.0, 10.0)),
        feature("A", polygon(0.0, 0.0, 10.0, 10.0)),
        feature("M", polygon(0.0, 0.0, 10.0, 10.0)),
    ]);

    let response = provider.search_sar(&aoi_query(3), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-Z.zip"), Some("S1-A.zip"), Some("S1-M.zip")]
    );

    let mut lexicographic = file_names(&response);
    lexicographic.sort_unstable();
    assert_eq!(
        lexicographic,
        [Some("S1-A.zip"), Some("S1-M.zip"), Some("S1-Z.zip")],
        "the anti-lexical fixture must fail if a key sort replaces provider order"
    );
}

#[test]
fn antimeridian_equal_coverage_remains_in_provider_order_after_alignment() {
    let provider = provider(vec![
        feature("NORMAL", polygon(166.0, -45.0, 168.0, -35.0)),
        feature("CROSS", polygon(178.0, -45.0, -178.0, -35.0)),
        feature("OUTSIDE", polygon(0.0, 0.0, 1.0, 1.0)),
    ]);

    let response = provider
        .search_sar(&antimeridian_aoi_query(2), &|| false)
        .unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-NORMAL.zip"), Some("S1-CROSS.zip")],
        "equal local-domain overlap must preserve the Provider order across the antimeridian"
    );
}

#[test]
fn antimeridian_higher_coverage_outranks_source_order_when_footprint_extends_aoi_domain() {
    let provider = provider(vec![
        feature("LOW", polygon(166.0, -45.0, 168.0, -35.0)),
        feature("HIGH", polygon(160.0, -45.0, 170.0, -35.0)),
        feature("OUTSIDE", polygon(0.0, 0.0, 1.0, 1.0)),
    ]);

    let response = provider
        .search_sar(&antimeridian_aoi_query(2), &|| false)
        .unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-HIGH.zip"), Some("S1-LOW.zip")],
        "coverage priority must win even when the best footprint extends west of the AOI reference interval"
    );
}

#[test]
fn global_aoi_sums_both_periodic_footprint_copies_before_ranking() {
    let provider = provider(vec![
        feature("LOW", polygon(-180.0, 0.0, -165.0, 10.0)),
        feature("CROSSING", polygon(170.0, 0.0, -170.0, 10.0)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(-180.0, 0.0, 180.0, 10.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-CROSSING.zip")]);
}

#[test]
fn global_aoi_ranks_full_world_component_unions_by_each_component_copy() {
    let provider = provider(vec![
        feature("LOW", polygon(-180.0, 0.0, -80.0, 1.0)),
        feature("FULL-COMPONENTS", full_world_component_union()),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(-180.0, 0.0, 180.0, 5.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-FULL-COMPONENTS.zip")]);
}

#[test]
fn projected_aoi_component_overlap_disables_derived_filtering_and_ranking() {
    let provider = provider(vec![
        feature("FIRST", polygon(0.0, 0.0, 1.0, 1.0)),
        feature("LOCAL", polygon(172.0, 2.0, 173.0, 3.0)),
    ]);

    let response = provider
        .search_sar(&geometry_aoi_query(periodic_overlap_geometry(), 2), &|| {
            false
        })
        .unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-FIRST.zip"), Some("S1-LOCAL.zip")]
    );
}

#[test]
fn projected_footprint_component_overlap_scores_zero_without_double_counting() {
    let provider = provider(vec![
        feature("AMBIGUOUS", periodic_overlap_geometry()),
        feature("VALID", polygon(170.0, 0.0, -175.0, 10.0)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(170.0, 0.0, -170.0, 10.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-VALID.zip")]);
}

#[test]
fn projected_single_component_self_cross_scores_zero_before_boolean_coverage() {
    let provider = provider(vec![
        feature("AMBIGUOUS", projection_self_cross_geometry()),
        feature("VALID", polygon(-180.0, 0.0, -179.9, 0.1)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(-180.0, 0.0, 180.0, 4.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-VALID.zip")]);
}

#[test]
fn projected_ring_work_exhaustion_scores_footprint_zero() {
    let provider = provider(vec![
        feature("ADVERSARIAL", dense_crossing_rectangle(400)),
        feature("VALID", polygon(170.0, 0.0, 170.1, 0.1)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(170.0, 0.0, -170.0, 10.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-VALID.zip")]);
}

#[test]
fn projected_relate_pair_work_exhaustion_scores_footprint_zero() {
    let provider = provider(vec![
        feature("ADVERSARIAL", hole_rich_disjoint_components()),
        feature("VALID", polygon(0.0, 0.0, 1.0, 1.0)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(0.0, 0.0, 10.0, 10.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-VALID.zip")]);
}

#[test]
fn boundary_only_edge_point_and_seam_footprints_score_zero_coverage() {
    for (aoi, boundary, positive) in [
        (
            polygon(0.0, 0.0, 10.0, 10.0),
            polygon(10.0, 2.0, 20.0, 8.0),
            polygon(0.0, 0.0, 0.1, 0.1),
        ),
        (
            polygon(0.0, 0.0, 10.0, 10.0),
            polygon(10.0, 10.0, 20.0, 20.0),
            polygon(0.0, 0.0, 0.1, 0.1),
        ),
        (
            polygon(170.0, 0.0, 180.0, 10.0),
            polygon(-180.0, 2.0, -170.0, 8.0),
            polygon(170.0, 0.0, 170.1, 0.1),
        ),
        (
            polygon(-180.0, 0.0, -170.0, 10.0),
            polygon(170.0, 2.0, 180.0, 8.0),
            polygon(-180.0, 0.0, -179.9, 0.1),
        ),
    ] {
        let provider = provider(vec![
            feature("BOUNDARY", boundary),
            feature("POSITIVE", positive),
        ]);

        let response = provider
            .search_sar(&geometry_aoi_query(aoi, 1), &|| false)
            .unwrap();

        assert_eq!(file_names(&response), [Some("S1-POSITIVE.zip")]);
    }
}

#[test]
fn full_world_footprint_uses_both_positive_area_copies_for_a_seam_aoi() {
    let provider = provider(vec![
        feature("LOCAL", polygon(170.0, 0.0, 180.0, 10.0)),
        feature("GLOBAL", polygon(-180.0, 0.0, 180.0, 10.0)),
    ]);

    let response = provider
        .search_sar(
            &geometry_aoi_query(polygon(170.0, 0.0, -170.0, 10.0), 1),
            &|| false,
        )
        .unwrap();

    assert_eq!(file_names(&response), [Some("S1-GLOBAL.zip")]);
}

#[test]
fn one_window_antimeridian_seam_aliases_use_their_local_domain() {
    for (aoi, local) in [
        (
            polygon(170.0, 0.0, -180.0, 10.0),
            polygon(172.0, 0.0, 178.0, 10.0),
        ),
        (
            polygon(-170.0, 0.0, 180.0, 10.0),
            polygon(-178.0, 0.0, -172.0, 10.0),
        ),
    ] {
        let provider = provider(vec![
            feature("GREENWICH", polygon(0.0, 0.0, 1.0, 10.0)),
            feature("LOCAL", local),
        ]);

        let response = provider
            .search_sar(&geometry_aoi_query(aoi, 1), &|| false)
            .unwrap();

        assert_eq!(file_names(&response), [Some("S1-LOCAL.zip")]);
    }
}

#[test]
fn greenwich_aoi_does_not_admit_a_remote_antimeridian_world_copy() {
    let provider = provider(vec![
        feature("SEAM", polygon(175.0, 0.0, -175.0, 10.0)),
        feature("LOCAL", polygon(0.0, 0.0, 2.0, 10.0)),
    ]);
    let query = geometry_aoi_query(polygon(-10.0, 0.0, 10.0, 10.0), 1);

    let response = provider.search_sar(&query, &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-LOCAL.zip")]);
}

#[test]
fn invalid_topology_footprints_cannot_outrank_a_valid_overlap() {
    let invalid_bow_tie = polygon_with_rings(vec![vec![
        (0.0, 0.0),
        (10.0, 8.0),
        (0.0, 10.0),
        (6.0, 0.0),
        (0.0, 0.0),
    ]]);
    let invalid_outside_hole = polygon_with_rings(vec![
        ring(0.0, 0.0, 10.0, 10.0),
        ring(20.0, 20.0, 21.0, 21.0),
    ]);
    let invalid_crossing_hole =
        polygon_with_rings(vec![ring(0.0, 0.0, 10.0, 10.0), ring(8.0, 2.0, 12.0, 8.0)]);
    let invalid_overlapping_holes = polygon_with_rings(vec![
        ring(0.0, 0.0, 10.0, 10.0),
        ring(1.0, 1.0, 6.0, 6.0),
        ring(4.0, 4.0, 9.0, 9.0),
    ]);
    let invalid_overlapping_components = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 10.0, 10.0)],
        vec![ring(2.0, 2.0, 8.0, 8.0)],
    ]);
    let provider = provider(vec![
        feature("BOW-TIE", invalid_bow_tie),
        feature("OUTSIDE-HOLE", invalid_outside_hole),
        feature("CROSSING-HOLE", invalid_crossing_hole),
        feature("OVERLAPPING-HOLES", invalid_overlapping_holes),
        feature("OVERLAPPING-COMPONENTS", invalid_overlapping_components),
        feature("VALID", polygon(0.0, 0.0, 2.0, 10.0)),
    ]);

    let response = provider.search_sar(&aoi_query(3), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [
            Some("S1-VALID.zip"),
            Some("S1-BOW-TIE.zip"),
            Some("S1-OUTSIDE-HOLE.zip"),
        ],
        "invalid topology must receive a zero score, leaving the all-zero suffix in provider order"
    );
}

#[test]
fn valid_holes_and_disjoint_multipolygons_keep_their_real_coverage_priority() {
    let legal_hole = polygon_with_rings(vec![ring(0.0, 0.0, 10.0, 10.0), ring(1.0, 1.0, 4.0, 4.0)]);
    let disjoint_components = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 4.0, 5.0)],
        vec![ring(6.0, 0.0, 10.0, 5.0)],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("DISJOINT", disjoint_components),
        feature("LEGAL-HOLE", legal_hole),
    ]);

    let response = provider.search_sar(&aoi_query(2), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-LEGAL-HOLE.zip"), Some("S1-DISJOINT.zip")]
    );
}

#[test]
fn invalid_raw_bow_tie_aoi_cannot_fall_back_to_an_unrelated_bbox() {
    let invalid_aoi = polygon_with_rings(vec![vec![
        (0.0, 0.0),
        (10.0, 8.0),
        (0.0, 10.0),
        (6.0, 0.0),
        (0.0, 0.0),
    ]]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
    ]);

    let error = provider
        .search_sar(
            &aoi_query_with_geometry_and_bbox(invalid_aoi, standard_bbox(), 1),
            &|| false,
        )
        .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
}

#[test]
fn invalid_raw_multipolygon_aoi_cannot_fall_back_to_an_unrelated_bbox() {
    let invalid_aoi = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 10.0, 10.0)],
        vec![ring(2.0, 2.0, 8.0, 8.0)],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
    ]);

    let error = provider
        .search_sar(
            &aoi_query_with_geometry_and_bbox(invalid_aoi, standard_bbox(), 1),
            &|| false,
        )
        .unwrap_err();

    assert_eq!(error.code, "ASF_QUERY_INVALID");
}

#[test]
fn bbox_without_raw_aoi_still_prioritizes_coverage() {
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
    ]);

    let response = provider.search_sar(&bbox_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-HIGH.zip")]);
}

#[test]
fn point_touching_multipolygon_components_remain_valid() {
    let point_touching = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 4.0, 4.0)],
        vec![ring(4.0, 4.0, 8.0, 8.0)],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("POINT-TOUCH", point_touching),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-POINT-TOUCH.zip")]);
}

#[test]
fn vertex_on_another_components_edge_remains_valid() {
    let t_touching = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 4.0, 4.0)],
        vec![vec![
            (2.0, 4.0),
            (6.0, 5.0),
            (6.0, 8.0),
            (2.0, 8.0),
            (2.0, 4.0),
        ]],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("T-TOUCH", t_touching),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-T-TOUCH.zip")]);
}

#[test]
fn shared_edges_and_proper_crossings_are_zero_score() {
    let shared_edge = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 4.0, 4.0)],
        vec![ring(4.0, 0.0, 8.0, 4.0)],
    ]);
    let proper_crossing = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 8.0, 8.0)],
        vec![vec![
            (4.0, -2.0),
            (10.0, 4.0),
            (4.0, 10.0),
            (-2.0, 4.0),
            (4.0, -2.0),
        ]],
    ]);
    let provider = provider(vec![
        feature("SHARED-EDGE", shared_edge),
        feature("CROSSING", proper_crossing),
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("MISSING", Value::Null),
    ]);

    let response = provider.search_sar(&aoi_query(3), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [
            Some("S1-LOW.zip"),
            Some("S1-SHARED-EDGE.zip"),
            Some("S1-CROSSING.zip"),
        ]
    );
}

#[test]
fn island_inside_another_components_hole_remains_valid() {
    let island_in_hole = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 10.0, 10.0), ring(2.0, 2.0, 8.0, 8.0)],
        vec![ring(3.0, 3.0, 7.0, 7.0)],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("ISLAND", island_in_hole),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-ISLAND.zip")]);
}

#[test]
fn island_sharing_another_components_hole_edge_is_zero_score() {
    let shared_hole_edge = multipolygon_with_rings(vec![
        vec![ring(0.0, 0.0, 10.0, 10.0), ring(2.0, 2.0, 8.0, 8.0)],
        vec![ring(2.0, 3.0, 4.0, 5.0)],
    ]);
    let provider = provider(vec![
        feature("SHARED-HOLE-EDGE", shared_hole_edge),
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("MISSING", Value::Null),
    ]);

    let response = provider.search_sar(&aoi_query(2), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-LOW.zip"), Some("S1-SHARED-HOLE-EDGE.zip")]
    );
}

#[test]
fn near_collinear_endpoint_touch_remains_valid() {
    let shared = (19.850257749638203, 46.29709338043669);
    let near_collinear = multipolygon_with_rings(vec![
        vec![vec![
            shared,
            (20.31970698357233, 46.76654261437082),
            (21.0, 46.0),
            shared,
        ]],
        vec![vec![
            shared,
            (-48.51001596420236, -22.063180333403878),
            (-48.0, -23.0),
            shared,
        ]],
    ]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("NEAR-COLLINEAR", near_collinear),
    ]);
    let query = aoi_query_with_geometry_and_bbox(
        polygon(-60.0, -30.0, 30.0, 50.0),
        json!({"west": -60.0, "east": 30.0, "south": -30.0, "north": 50.0, "crs": "EPSG:4326"}),
        1,
    );

    let response = provider.search_sar(&query, &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-NEAR-COLLINEAR.zip")]);
}

#[test]
fn oversized_ring_disables_reranking_before_quadratic_validation() {
    let oversized = polygon_with_rings(vec![dense_ring(
        0.0,
        0.0,
        10.0,
        10.0,
        MAX_POLYGONAL_COORDS_PER_RING - 2,
    )]);
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("OVERSIZED", oversized),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-LOW.zip")]);
}

#[test]
fn total_coordinate_limit_disables_reranking_before_quadratic_validation() {
    let ring_positions = MAX_POLYGONAL_TOTAL_COORDS / 3 + 1;
    let over_total = multipolygon_with_rings(
        (0..3)
            .map(|index| {
                let west = index as f64 * 3.2;
                vec![dense_ring(west, 0.0, west + 2.5, 10.0, ring_positions - 3)]
            })
            .collect(),
    );
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("OVER-TOTAL", over_total),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-LOW.zip")]);
}

#[test]
fn point_and_null_geometries_cannot_outrank_a_positive_polygon() {
    let provider = provider(vec![
        feature("POINT", point(5.0, 5.0)),
        feature("MISSING", Value::Null),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
    ]);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert_eq!(file_names(&response), [Some("S1-HIGH.zip")]);
    assert_eq!(
        provider.transport().geojson_params.lock().unwrap()[0]["maxResults"],
        "51"
    );
}

#[test]
fn missing_aoi_preserves_provider_order() {
    let provider = provider(vec![
        feature("LOW", polygon(0.0, 0.0, 2.0, 10.0)),
        feature("MISSING", Value::Null),
        feature("HIGH", polygon(0.0, 0.0, 10.0, 10.0)),
        feature("MEDIUM", polygon(0.0, 0.0, 6.0, 10.0)),
    ]);

    let response = provider
        .search_sar(&query_without_aoi(2), &|| false)
        .unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-LOW.zip"), Some("S1-MISSING.zip")]
    );
}

#[test]
fn unusable_footprints_preserve_provider_order() {
    let provider = provider(vec![
        feature("MISSING", Value::Null),
        feature("POINT-A", point(2.0, 2.0)),
        feature("POINT-B", point(8.0, 8.0)),
    ]);

    let response = provider.search_sar(&aoi_query(2), &|| false).unwrap();

    assert_eq!(
        file_names(&response),
        [Some("S1-MISSING.zip"), Some("S1-POINT-A.zip")]
    );
}

#[test]
fn target_one_aoi_search_does_not_refill_past_metadata_cap() {
    let provider = provider_with_count((0..51).map(|_| unmappable_feature()).collect(), None);

    let response = provider.search_sar(&aoi_query(1), &|| false).unwrap();

    assert!(response.products.is_empty());
    let params = provider.transport().geojson_params.lock().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0]["maxResults"], "51");
}
