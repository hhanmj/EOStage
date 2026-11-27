use geo::{BoundingRect, Coord, LineString, MultiPolygon, Polygon};
use insar_core::polygonal_topology::MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES;
use insar_core::{
    align_polygonal_to_longitude_domain, align_polygonal_to_longitude_domain_render,
    align_polygonal_to_longitude_reference, validated_authoritative_polygonal_geojson,
    wrapped_aoi_envelope, BBox, WrappedAoiEnvelope, MAX_PROJECTED_RENDER_LONGITUDE,
};
use serde_json::{json, Map, Value};

fn fixture_case(id: &str) -> Value {
    let root: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/geospatial/antimeridian_aoi_cases.json"
    ))
    .unwrap();
    root["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == id)
        .unwrap()
        .clone()
}

fn fixture(id: &str) -> Map<String, Value> {
    fixture_case(id)["geometry"].as_object().unwrap().clone()
}

fn global_fixture() -> Map<String, Value> {
    serde_json::json!({
        "type": "Polygon",
        "coordinates": [[[-180,-90],[180,-90],[180,90],[-180,90],[-180,-90]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn expected_bbox(value: &Value) -> BBox {
    let coordinates = value.as_array().unwrap();
    BBox {
        west: coordinates[0].as_f64().unwrap(),
        south: coordinates[1].as_f64().unwrap(),
        east: coordinates[2].as_f64().unwrap(),
        north: coordinates[3].as_f64().unwrap(),
        crs: "EPSG:4326".into(),
    }
}

fn polygonal_geometry(coordinates: &[(f64, f64)]) -> MultiPolygon<f64> {
    MultiPolygon(vec![Polygon::new(
        LineString::new(coordinates.iter().map(|&(x, y)| Coord { x, y }).collect()),
        Vec::new(),
    )])
}

fn polygonal_coords(geometry: &MultiPolygon<f64>) -> Vec<Coord<f64>> {
    geometry
        .0
        .iter()
        .flat_map(|polygon| {
            std::iter::once(polygon.exterior())
                .chain(polygon.interiors())
                .flat_map(|ring| ring.0.iter().copied())
        })
        .collect()
}

fn assert_projection_covers_exact_geometry(
    geometry: &MultiPolygon<f64>,
    envelope: &WrappedAoiEnvelope,
) {
    let aligned =
        align_polygonal_to_longitude_domain(geometry, envelope.display_west, envelope.display_east)
            .unwrap();
    for coord in polygonal_coords(&aligned) {
        assert!(
            coord.x >= envelope.display_west && coord.x <= envelope.display_east,
            "aligned longitude {} lies outside exact display domain [{}, {}]",
            coord.x,
            envelope.display_west,
            envelope.display_east
        );
    }
    for coord in polygonal_coords(geometry) {
        assert!(
            envelope
                .request_windows
                .iter()
                .any(|window| source_longitude_is_covered(coord.x, window)),
            "source longitude {} is not covered by any exact request window",
            coord.x
        );
    }
}

fn source_longitude_is_covered(longitude: f64, window: &BBox) -> bool {
    (longitude >= window.west && longitude <= window.east)
        || (longitude == -180.0 && 180.0 >= window.west && 180.0 <= window.east)
        || (longitude == 180.0 && -180.0 >= window.west && -180.0 <= window.east)
}

#[test]
fn native_render_longitude_contract_accepts_541_and_rejects_901() {
    let seam = polygonal_geometry(&[
        (179.0, 0.0),
        (-179.0, 0.0),
        (-179.0, 1.0),
        (179.0, 1.0),
        (179.0, 0.0),
    ]);
    let (render, bbox) = align_polygonal_to_longitude_domain_render(&seam, 539.0, 541.0)
        .expect("native seam projection can reach the legal 541-degree edge");
    assert_eq!(bbox, [539.0, 0.0, 541.0, 1.0]);
    assert!(render["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|polygon| polygon.as_array().unwrap())
        .flat_map(|ring| ring.as_array().unwrap())
        .all(|position| {
            position[0].as_f64().unwrap() >= -MAX_PROJECTED_RENDER_LONGITUDE
                && position[0].as_f64().unwrap() <= MAX_PROJECTED_RENDER_LONGITUDE
        }));
    assert!(align_polygonal_to_longitude_domain_render(&seam, 899.0, 901.0).is_none());
}

#[test]
fn reference_projection_composes_component_and_reference_world_shifts() {
    let reverse_seam = polygonal_geometry(&[
        (-179.0, 0.0),
        (179.0, 0.0),
        (179.0, 1.0),
        (-179.0, 1.0),
        (-179.0, 0.0),
    ]);
    let aligned = align_polygonal_to_longitude_reference(&reverse_seam, 179.5, 539.25)
        .expect("the reverse seam needs both component+reference copies");
    assert_eq!(aligned.as_slice().len(), 2);
    let coordinates = aligned
        .as_slice()
        .iter()
        .map(|polygon| {
            polygon
                .exterior()
                .0
                .iter()
                .map(|coordinate| coordinate.x)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        coordinates,
        vec![
            vec![181.0, 179.0, 179.0, 181.0, 181.0],
            vec![541.0, 539.0, 539.0, 541.0, 541.0]
        ]
    );
    assert!(polygonal_coords(&MultiPolygon(aligned.as_slice().to_vec()))
        .into_iter()
        .all(|coordinate| coordinate.x.abs() <= MAX_PROJECTED_RENDER_LONGITUDE));
}

fn assert_only_integer_world_shifts(source: &MultiPolygon<f64>, aligned: &MultiPolygon<f64>) {
    assert_eq!(source.0.len(), aligned.0.len());
    for (source_polygon, aligned_polygon) in source.0.iter().zip(&aligned.0) {
        assert_eq!(
            source_polygon.interiors().len(),
            aligned_polygon.interiors().len()
        );
        for (source_ring, aligned_ring) in std::iter::once(source_polygon.exterior())
            .chain(source_polygon.interiors())
            .zip(std::iter::once(aligned_polygon.exterior()).chain(aligned_polygon.interiors()))
        {
            assert_eq!(source_ring.0.len(), aligned_ring.0.len());
            for (source_coord, aligned_coord) in source_ring.0.iter().zip(&aligned_ring.0) {
                assert_eq!(source_coord.y, aligned_coord.y);
                let world_shift = ((aligned_coord.x - source_coord.x) / 360.0).round();
                assert_eq!(
                    aligned_coord.x,
                    source_coord.x + world_shift * 360.0,
                    "aligned longitude must preserve the exact source coordinate modulo 360"
                );
            }
        }
    }
}

fn assert_all_rings_closed(geometry: &MultiPolygon<f64>) {
    assert!(geometry.0.iter().all(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .all(|ring| !ring.0.is_empty() && ring.0.first() == ring.0.last())
    }));
}

fn quadrilateral_ring_variants(vertices: [[f64; 2]; 4]) -> Vec<Vec<[f64; 2]>> {
    [false, true]
        .into_iter()
        .flat_map(|reversed| {
            let mut ordered = vertices.to_vec();
            if reversed {
                ordered.reverse();
            }
            (0..ordered.len()).map(move |rotation| {
                let mut ring = ordered.clone();
                ring.rotate_left(rotation);
                ring.push(ring[0]);
                ring
            })
        })
        .collect()
}

fn rectangle_ring_variants(a: f64, b: f64) -> Vec<Vec<[f64; 2]>> {
    quadrilateral_ring_variants([[a, 0.0], [b, 0.0], [b, 1.0], [a, 1.0]])
}

fn many_tiny_components(component_count: usize) -> MultiPolygon<f64> {
    let polygons = (0..component_count)
        .map(|index| {
            let column = (index % 512) as f64;
            let row = (index / 512) as f64;
            let west = -120.123_456_789_012_3 + column * 0.000_001_234_567;
            let south = 30.123_456_789_012_3 + row * 0.000_001_234_567;
            let east = west + 0.000_000_4;
            let north = south + 0.000_000_4;
            Polygon::new(
                LineString::new(vec![
                    Coord { x: west, y: south },
                    Coord { x: east, y: south },
                    Coord { x: east, y: north },
                    Coord { x: west, y: north },
                    Coord { x: west, y: south },
                ]),
                Vec::new(),
            )
        })
        .collect();
    MultiPolygon(polygons)
}

fn concentric_annuli(component_count: usize) -> MultiPolygon<f64> {
    let ring = |radius: f64| {
        LineString::new(vec![
            Coord {
                x: -radius,
                y: -radius,
            },
            Coord {
                x: radius,
                y: -radius,
            },
            Coord {
                x: radius,
                y: radius,
            },
            Coord {
                x: -radius,
                y: radius,
            },
            Coord {
                x: -radius,
                y: -radius,
            },
        ])
    };
    MultiPolygon(
        (0..component_count)
            .map(|index| {
                let outer = 80.0 - index as f64 * 0.075;
                Polygon::new(ring(outer), vec![ring(outer - 0.025)])
            })
            .collect(),
    )
}

#[test]
fn reference_alignment_applies_decimal_world_shifts_once_for_every_ring_order() {
    for ring in
        quadrilateral_ring_variants([[179.95, 0.0], [-0.1, 0.0], [-0.1, 1.0], [179.95, 1.0]])
    {
        let coordinates = ring.into_iter().map(|[x, y]| (x, y)).collect::<Vec<_>>();
        let source = polygonal_geometry(&coordinates);

        let aligned = align_polygonal_to_longitude_reference(&source, -0.100_000_000_000_01, 1.0)
            .expect("valid decimal reference domain");
        let aligned_geometry = MultiPolygon(aligned.as_slice().to_vec());

        assert_eq!(aligned.as_slice().len(), 1);
        assert_only_integer_world_shifts(&source, &aligned_geometry);
        assert!(
            polygonal_coords(&aligned_geometry)
                .into_iter()
                .any(|coord| coord.x == -0.1),
            "the raw -0.1 endpoint must survive one combined integer-world projection"
        );
    }
}

#[test]
fn reference_alignment_returns_both_periodic_copies_needed_by_wide_domains() {
    let source = polygonal_geometry(&[
        (170.0, 0.0),
        (-170.0, 0.0),
        (-170.0, 1.0),
        (170.0, 1.0),
        (170.0, 0.0),
    ]);

    for (west, east, expected_overlap) in [
        (-180.0, 180.0, 20.0),
        (-179.0, 179.0, 18.0),
        (-175.0, 175.0, 10.0),
    ] {
        let reference = polygonal_geometry(&[
            (west, 0.0),
            (east, 0.0),
            (east, 1.0),
            (west, 1.0),
            (west, 0.0),
        ]);

        let aligned = align_polygonal_to_longitude_reference(&source, west, east)
            .expect("valid wide reference domain");

        assert_eq!(
            aligned.as_slice().len(),
            2,
            "both adjacent periodic copies are required for [{west}, {east}]"
        );
        assert_eq!(aligned.overlap_area(&reference), Some(expected_overlap));
    }
}

#[test]
fn reference_alignment_excludes_boundary_only_periodic_copies() {
    let source = polygonal_geometry(&[
        (-180.0, 0.0),
        (180.0, 0.0),
        (180.0, 1.0),
        (-180.0, 1.0),
        (-180.0, 0.0),
    ]);

    let aligned = align_polygonal_to_longitude_reference(&source, -180.0, 180.0)
        .expect("valid global reference domain");

    assert_eq!(aligned.as_slice().len(), 1);
    assert_eq!(aligned.overlap_area(&source), Some(360.0));

    let seam_reference = polygonal_geometry(&[
        (170.0, 0.0),
        (190.0, 0.0),
        (190.0, 1.0),
        (170.0, 1.0),
        (170.0, 0.0),
    ]);
    let seam_aligned = align_polygonal_to_longitude_reference(&source, 170.0, 190.0)
        .expect("shared 180-degree boundaries do not overlap copy interiors");
    assert_eq!(seam_aligned.as_slice().len(), 2);
    assert_eq!(seam_aligned.overlap_area(&seam_reference), Some(20.0));
}

#[test]
fn reference_alignment_projects_full_world_component_unions_per_component() {
    let component = |coordinates: &[(f64, f64)]| {
        Polygon::new(
            LineString::new(coordinates.iter().map(|&(x, y)| Coord { x, y }).collect()),
            Vec::new(),
        )
    };
    let source = MultiPolygon(vec![
        component(&[
            (170.0, 0.0),
            (-170.0, 0.0),
            (-170.0, 1.0),
            (170.0, 1.0),
            (170.0, 0.0),
        ]),
        component(&[
            (-170.0, 2.0),
            (0.0, 2.0),
            (0.0, 3.0),
            (-170.0, 3.0),
            (-170.0, 2.0),
        ]),
        component(&[
            (0.0, 4.0),
            (170.0, 4.0),
            (170.0, 5.0),
            (0.0, 5.0),
            (0.0, 4.0),
        ]),
    ]);
    let reference = polygonal_geometry(&[
        (-180.0, 0.0),
        (180.0, 0.0),
        (180.0, 5.0),
        (-180.0, 5.0),
        (-180.0, 0.0),
    ]);

    let aligned = align_polygonal_to_longitude_reference(&source, -180.0, 180.0)
        .expect("each valid component must choose its own canonical base copy");

    assert_eq!(aligned.as_slice().len(), 4);
    assert_eq!(aligned.overlap_area(&reference), Some(360.0));
}

#[test]
fn reference_alignment_rejects_component_interiors_that_overlap_only_after_projection() {
    let component = |coordinates: &[(f64, f64)]| {
        Polygon::new(
            LineString::new(coordinates.iter().map(|&(x, y)| Coord { x, y }).collect()),
            Vec::new(),
        )
    };
    let source = MultiPolygon(vec![
        component(&[
            (170.0, 0.0),
            (-170.0, 0.0),
            (-170.0, 10.0),
            (170.0, 10.0),
            (170.0, 0.0),
        ]),
        component(&[
            (171.0, 1.0),
            (179.0, 1.0),
            (179.0, 9.0),
            (171.0, 9.0),
            (171.0, 1.0),
        ]),
    ]);

    assert!(
        align_polygonal_to_longitude_reference(&source, 170.0, 190.0).is_none(),
        "derived component overlap must fail closed before Boolean coverage"
    );
}

#[test]
fn alignment_rejects_a_single_component_that_self_crosses_only_after_unwrap() {
    let source = polygonal_geometry(&[
        (-120.0, 1.0),
        (-60.0, 2.0),
        (170.0, 0.0),
        (-120.0, 4.0),
        (-120.0, 1.0),
    ]);
    assert!(
        align_polygonal_to_longitude_domain(&source, -190.0, -60.0).is_none(),
        "derived AOI projection must reject its lifted self-crossing"
    );
    assert!(
        align_polygonal_to_longitude_reference(&source, -180.0, 180.0).is_none(),
        "derived footprint projection must reject its lifted self-crossing"
    );
    assert!(
        wrapped_aoi_envelope(&source).is_none(),
        "an invalid lifted component cannot authorize derived request windows"
    );
}

#[test]
fn projected_topology_budget_fails_closed_for_concentric_pair_work() {
    let source = concentric_annuli(1_000);

    assert!(
        align_polygonal_to_longitude_domain(&source, -80.0, 80.0).is_none(),
        "derived topology must stop before unbounded concentric pair work"
    );
}

#[test]
fn ordinary_mainland_geometry_keeps_its_single_fixed_window() {
    let case = fixture_case("hubei");
    let geometry =
        validated_authoritative_polygonal_geojson(case["geometry"].as_object().unwrap()).unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (
            envelope.display_west,
            envelope.south,
            envelope.display_east,
            envelope.north,
        ),
        (108.0, 29.0, 116.0, 34.0)
    );
    assert!(!envelope.crosses_antimeridian);
    assert_eq!(
        envelope.request_windows,
        case["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(expected_bbox)
            .collect::<Vec<_>>()
    );
}

#[test]
fn new_zealand_uses_two_local_windows_instead_of_a_world_bbox() {
    let case = fixture_case("new-zealand");
    let geometry =
        validated_authoritative_polygonal_geojson(case["geometry"].as_object().unwrap()).unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert!(envelope.crosses_antimeridian);
    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (166.0, 185.0)
    );
    assert_eq!(envelope.request_windows.len(), 2);
    assert!(envelope
        .request_windows
        .iter()
        .all(|bbox| bbox.west < bbox.east && bbox.validate().is_ok()));
    assert_eq!(
        envelope.request_windows,
        case["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(expected_bbox)
            .collect::<Vec<_>>()
    );
    assert!(envelope.display_east - envelope.display_west < 30.0);
}

#[test]
fn native_aoi_render_uses_one_strict_display_domain_and_exact_bbox() {
    let case = fixture_case("new-zealand");
    let geometry =
        validated_authoritative_polygonal_geojson(case["geometry"].as_object().unwrap()).unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();
    let (_render, bbox) = align_polygonal_to_longitude_domain_render(
        &geometry.geometry,
        envelope.display_west,
        envelope.display_east,
    )
    .unwrap();
    assert_eq!(
        bbox,
        [
            envelope.display_west,
            envelope.south,
            envelope.display_east,
            envelope.north,
        ]
    );
}

#[test]
fn native_render_full_world_shapes_keep_raw_endpoint_direction_across_worlds() {
    let global = validated_authoritative_polygonal_geojson(&global_fixture()).unwrap();

    let (base_render, base_bbox) =
        align_polygonal_to_longitude_domain_render(&global.geometry, -180.0, 180.0)
            .expect("the exact [-180, 180] producer shape is valid");
    assert_eq!(base_bbox, [-180.0, -90.0, 180.0, 90.0]);
    assert_eq!(base_render["coordinates"][0][0][0][0], json!(-180.0));
    assert_eq!(base_render["coordinates"][0][0][1][0], json!(180.0));

    let (adjacent_render, adjacent_bbox) =
        align_polygonal_to_longitude_domain_render(&global.geometry, 180.0, 540.0)
            .expect("the same full-world producer shape may be rendered in an adjacent world");
    assert_eq!(adjacent_bbox, [180.0, -90.0, 540.0, 90.0]);
    assert_eq!(adjacent_render["coordinates"][0][0][0][0], json!(180.0));
    assert_eq!(adjacent_render["coordinates"][0][0][1][0], json!(540.0));

    let near_global = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[
                -179.99999999999997, -90.0,
            ], [179.99999999999997, -90.0], [179.99999999999997, 90.0],
                [-179.99999999999997, 90.0], [-179.99999999999997, -90.0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let near_envelope = wrapped_aoi_envelope(&near_global.geometry).unwrap();
    assert!(near_envelope.display_east - near_envelope.display_west < 360.0);
    assert!(!(near_envelope.display_west == -180.0 && near_envelope.display_east == 180.0));
    let (_, near_bbox) = align_polygonal_to_longitude_domain_render(
        &near_global.geometry,
        near_envelope.display_west,
        near_envelope.display_east,
    )
    .expect("near-global geometry remains a bounded ordinary projection");
    assert!(!(near_bbox[0] == -180.0 && near_bbox[2] == 180.0));
}

#[test]
fn one_ring_with_consecutive_coordinates_across_180_is_unwrapped_locally() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[179,10],[-179,10],[-179,12],[179,12],[179,10]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (179.0, 181.0)
    );
    assert_eq!(
        envelope.request_windows,
        vec![
            BBox {
                west: 179.0,
                south: 10.0,
                east: 180.0,
                north: 12.0,
                crs: "EPSG:4326".into(),
            },
            BBox {
                west: -180.0,
                south: 10.0,
                east: -179.0,
                north: 12.0,
                crs: "EPSG:4326".into(),
            },
        ]
    );
}

#[test]
fn exact_180_ties_are_directional_closed_and_rotation_invariant() {
    let variants = [
        json!([[[-90, 0], [90, 0], [90, 1], [-90, 1], [-90, 0]]]),
        json!([[[-90, 0], [-90, 1], [90, 1], [90, 0], [-90, 0]]]),
        json!([[[90, 0], [90, 1], [-90, 1], [-90, 0], [90, 0]]]),
    ];
    let expected_window = BBox {
        west: -90.0,
        south: 0.0,
        east: 90.0,
        north: 1.0,
        crs: "EPSG:4326".into(),
    };

    for coordinates in variants {
        let geometry = validated_authoritative_polygonal_geojson(
            json!({"type": "Polygon", "coordinates": coordinates})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();
        assert_eq!(
            (envelope.display_west, envelope.display_east),
            (-90.0, 90.0)
        );
        assert_eq!(envelope.request_windows, vec![expected_window.clone()]);

        let aligned = align_polygonal_to_longitude_domain(
            &geometry.geometry,
            envelope.display_west,
            envelope.display_east,
        )
        .unwrap();
        assert_all_rings_closed(&aligned);
        assert_only_integer_world_shifts(&geometry.geometry, &aligned);
        assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
    }
}

#[test]
fn decimal_exact_180_ties_preserve_source_identity_for_every_ring_order() {
    let a = -0.1;
    let b = 179.9;
    let expected_window = BBox {
        west: a,
        south: 0.0,
        east: b,
        north: 1.0,
        crs: "EPSG:4326".into(),
    };

    for ring in rectangle_ring_variants(a, b) {
        let geometry = validated_authoritative_polygonal_geojson(
            json!({"type": "Polygon", "coordinates": [ring]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let envelope = wrapped_aoi_envelope(&geometry.geometry)
            .expect("a decimal exact-180 ring must retain a closed integer-world lift");
        assert_eq!((envelope.display_west, envelope.display_east), (a, b));
        assert!(!envelope.crosses_antimeridian);
        assert_eq!(envelope.request_windows, vec![expected_window.clone()]);

        let aligned = align_polygonal_to_longitude_domain(
            &geometry.geometry,
            envelope.display_west,
            envelope.display_east,
        )
        .unwrap();
        assert_all_rings_closed(&aligned);
        assert_only_integer_world_shifts(&geometry.geometry, &aligned);
        assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
    }
}

#[test]
fn decimal_antimeridian_rotations_share_exact_single_addition_projection() {
    let a = -179.95;
    let b = 0.1;
    let expected_windows = vec![
        BBox {
            west: b,
            south: 0.0,
            east: 180.0,
            north: 1.0,
            crs: "EPSG:4326".into(),
        },
        BBox {
            west: -180.0,
            south: 0.0,
            east: a,
            north: 1.0,
            crs: "EPSG:4326".into(),
        },
    ];

    for ring in rectangle_ring_variants(a, b) {
        let geometry = validated_authoritative_polygonal_geojson(
            json!({"type": "Polygon", "coordinates": [ring]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();
        assert_eq!((envelope.display_west, envelope.display_east), (b, 180.05));
        assert!(envelope.crosses_antimeridian);
        assert_eq!(envelope.request_windows, expected_windows);

        let aligned = align_polygonal_to_longitude_domain(
            &geometry.geometry,
            envelope.display_west,
            envelope.display_east,
        )
        .unwrap();
        assert_all_rings_closed(&aligned);
        assert_only_integer_world_shifts(&geometry.geometry, &aligned);
        assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
    }
}

#[test]
fn seam_and_colliding_decimal_extrema_are_ring_order_invariant() {
    let a = -0.10000000000001;
    let b = -0.1;
    let expected_window = BBox {
        west: -180.0,
        south: 0.0,
        east: b,
        north: 1.0,
        crs: "EPSG:4326".into(),
    };
    let variants = quadrilateral_ring_variants([[180.0, 0.0], [a, 0.0], [b, 1.0], [180.0, 1.0]]);

    for ring in variants {
        let geometry = validated_authoritative_polygonal_geojson(
            json!({"type": "Polygon", "coordinates": [ring]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();
        assert_eq!((envelope.display_west, envelope.display_east), (-180.0, b));
        assert!(!envelope.crosses_antimeridian);
        assert_eq!(envelope.request_windows, vec![expected_window.clone()]);

        let aligned = align_polygonal_to_longitude_domain(
            &geometry.geometry,
            envelope.display_west,
            envelope.display_east,
        )
        .unwrap();
        assert_all_rings_closed(&aligned);
        assert_only_integer_world_shifts(&geometry.geometry, &aligned);
        assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
    }
}

#[test]
fn predecessor_of_180_near_global_diamond_is_not_promoted_to_full_world() {
    let predecessor = 179.99999999999997;
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[
                [-180,0],
                [0,0],
                [predecessor,1],
                [0,2],
                [-180,0]
            ]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (-180.0, predecessor)
    );
    assert!(!envelope.crosses_antimeridian);
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: -180.0,
            south: 0.0,
            east: predecessor,
            north: 2.0,
            crs: "EPSG:4326".into(),
        }]
    );
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn predecessor_to_negative_180_edge_is_local_not_explicitly_full_world() {
    let predecessor = 179.99999999999997;
    let expected_window = BBox {
        west: predecessor,
        south: 0.0,
        east: 180.0,
        north: 1.0,
        crs: "EPSG:4326".into(),
    };

    for ring in rectangle_ring_variants(-180.0, predecessor) {
        let geometry = validated_authoritative_polygonal_geojson(
            json!({"type": "Polygon", "coordinates": [ring]})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();
        assert_eq!(
            (envelope.display_west, envelope.display_east),
            (predecessor, 180.0)
        );
        assert!(!envelope.crosses_antimeridian);
        assert_eq!(envelope.request_windows, vec![expected_window.clone()]);
        assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
    }
}

#[test]
fn near_full_cover_places_boundary_component_by_exact_endpoints() {
    let west_successor = -179.99999999999997;
    let east_predecessor = 179.99999999999997;
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[
                    [-180,0],
                    [west_successor,0],
                    [west_successor,1],
                    [-180,1],
                    [-180,0]
                ]],
                [[
                    [west_successor,10],
                    [0,10],
                    [east_predecessor,11],
                    [0,12],
                    [west_successor,10]
                ]]
            ]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (-180.0, east_predecessor)
    );
    assert!(!envelope.crosses_antimeridian);
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: -180.0,
            south: 0.0,
            east: east_predecessor,
            north: 12.0,
            crs: "EPSG:4326".into(),
        }]
    );
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn exact_larger_wrap_gap_wins_when_rounded_gap_widths_tie() {
    let a_west = -90.00000000000004;
    let a_east = -89.99999999999999;
    let b_west = 89.99999999999996;
    let b_east = 90.0;
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[a_west,0],[a_east,0],[a_east,1],[a_west,1],[a_west,0]]],
                [[[b_west,2],[b_east,2],[b_east,3],[b_west,3],[b_west,2]]]
            ]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (a_west, b_east)
    );
    assert!(!envelope.crosses_antimeridian);
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: a_west,
            south: 0.0,
            east: b_east,
            north: 3.0,
            crs: "EPSG:4326".into(),
        }]
    );
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn subnanodegree_mainland_extreme_is_not_clipped_inward() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[0,0],[10,0],[10.0000000005,1],[0,1],[0,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    assert!(
        align_polygonal_to_longitude_domain(&geometry.geometry, 0.0, 10.0).is_none(),
        "alignment must not accept a domain that clips a subnanodegree extreme"
    );
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(envelope.display_west, 0.0);
    assert_eq!(envelope.display_east, 10.0000000005);
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn subnanodegree_antimeridian_extreme_is_not_clipped_inward() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[179,0],[-180,0],[-179.9999999995,1],[179,1],[179,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    assert!(
        align_polygonal_to_longitude_domain(&geometry.geometry, 179.0, 180.0).is_none(),
        "antimeridian alignment must not accept an inward-clipped domain"
    );
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(envelope.display_west, 179.0);
    assert_eq!(envelope.display_east, 180.0000000005);
    assert_eq!(envelope.request_windows.len(), 2);
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn western_hemisphere_decimal_extrema_survive_domain_canonicalization() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[-0.1,0],[-0.05,0],[-0.05,1],[-0.1,1],[-0.1,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry)
        .expect("ordinary western-hemisphere decimals must retain a valid envelope");

    assert!(!envelope.crosses_antimeridian);
    assert!(envelope.display_west <= -0.1);
    assert!(envelope.display_east >= -0.05);
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn normalized_hint_collision_does_not_erase_exact_source_width() {
    let source_west = -0.10000000000001;
    let source_east = -0.1;
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[
                [source_west,0],
                [source_east,0],
                [source_east,1],
                [source_west,1],
                [source_west,0]
            ]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry)
        .expect("normalized hint cardinality must not erase actual source width");

    assert_eq!(envelope.display_west, source_west);
    assert_eq!(envelope.display_east, source_east);
    assert!(!envelope.crosses_antimeridian);
    assert_eq!(envelope.request_windows.len(), 1);
    assert!(envelope.request_windows[0].west <= source_west);
    assert!(envelope.request_windows[0].east >= source_east);

    let aligned = align_polygonal_to_longitude_domain(
        &geometry.geometry,
        envelope.display_west,
        envelope.display_east,
    )
    .unwrap();
    let aligned_longitudes = polygonal_coords(&aligned)
        .into_iter()
        .map(|coord| coord.x)
        .collect::<Vec<_>>();
    assert!(aligned_longitudes.contains(&source_west));
    assert!(aligned_longitudes.contains(&source_east));
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn antimeridian_window_covers_decimal_source_extreme_after_wrapping() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[179.95,0],[-0.1000000005,0],[-0.1,1],[179.95,1],[179.95,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert!(envelope.crosses_antimeridian);
    assert_eq!(envelope.request_windows.len(), 2);
    assert!(envelope.request_windows[1].east >= -0.1);
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn disconnected_actual_intervals_choose_the_minimum_valid_union() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[-177,0],[-177,1],[-135,0],[-177,0]]],
                [[[-171,10],[15,11],[27,10],[-171,10]]]
            ]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry)
        .expect("actual component intervals have a valid 210-degree union");

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (15.0, 225.0)
    );
    assert_eq!(
        envelope.request_windows,
        vec![
            BBox {
                west: 15.0,
                south: 0.0,
                east: 180.0,
                north: 11.0,
                crs: "EPSG:4326".into(),
            },
            BBox {
                west: -180.0,
                south: 0.0,
                east: -135.0,
                north: 11.0,
                crs: "EPSG:4326".into(),
            },
        ]
    );
    let aligned = align_polygonal_to_longitude_domain(
        &geometry.geometry,
        envelope.display_west,
        envelope.display_east,
    )
    .unwrap();
    assert_only_integer_world_shifts(&geometry.geometry, &aligned);
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn negative_180_seam_alias_stays_a_local_single_window() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[170,0],[-180,0],[-180,1],[170,1],[170,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (170.0, 180.0)
    );
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: 170.0,
            south: 0.0,
            east: 180.0,
            north: 1.0,
            crs: "EPSG:4326".into(),
        }]
    );
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn positive_180_seam_alias_stays_a_local_single_window() {
    let geometry = validated_authoritative_polygonal_geojson(
        json!({
            "type": "Polygon",
            "coordinates": [[[-170,0],[180,0],[180,1],[-170,1],[-170,0]]]
        })
        .as_object()
        .unwrap(),
    )
    .unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (-180.0, -170.0)
    );
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: -180.0,
            south: 0.0,
            east: -170.0,
            north: 1.0,
            crs: "EPSG:4326".into(),
        }]
    );
    assert_projection_covers_exact_geometry(&geometry.geometry, &envelope);
}

#[test]
fn actual_interval_sweep_accepts_277_tiny_components() {
    let geometry = many_tiny_components(277);
    let envelope = wrapped_aoi_envelope(&geometry).unwrap();

    assert_eq!(envelope.request_windows.len(), 1);
    assert!(!envelope.crosses_antimeridian);
    assert_projection_covers_exact_geometry(&geometry, &envelope);
}

#[test]
fn actual_interval_sweep_accepts_near_authority_ceiling_tiny_components() {
    let geometry = many_tiny_components(38_000);
    let geojson = geojson::Geometry::new(geojson::Value::from(&geometry));
    let serialized_size = serde_json::to_vec(&geojson).unwrap().len();
    assert!(
        serialized_size >= MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES * 9 / 10,
        "fixture must exercise a near-ceiling authority, got {serialized_size} bytes"
    );
    assert!(serialized_size < MAX_AUTHORITATIVE_POLYGONAL_JSON_BYTES);

    let envelope = wrapped_aoi_envelope(&geometry).unwrap();
    assert_eq!(envelope.request_windows.len(), 1);
    assert!(!envelope.crosses_antimeridian);
    assert_projection_covers_exact_geometry(&geometry, &envelope);
}

#[test]
fn disconnected_components_align_to_the_same_continuous_display_domain() {
    let geometry = validated_authoritative_polygonal_geojson(&fixture("new-zealand")).unwrap();
    let aligned = align_polygonal_to_longitude_domain(&geometry.geometry, 166.0, 185.0).unwrap();
    let bounds = aligned.bounding_rect().unwrap();

    assert_eq!(
        (
            bounds.min().x,
            bounds.min().y,
            bounds.max().x,
            bounds.max().y
        ),
        (166.0, -47.0, 185.0, -34.0)
    );
    assert!(aligned.0.iter().all(|polygon| {
        std::iter::once(polygon.exterior())
            .chain(polygon.interiors())
            .all(|ring| {
                ring.0
                    .windows(2)
                    .all(|edge| (edge[1].x - edge[0].x).abs() <= 180.0)
            })
    }));
}

#[test]
fn global_geometry_is_not_invented_as_a_narrow_local_window() {
    let geometry = validated_authoritative_polygonal_geojson(&global_fixture()).unwrap();
    let envelope = wrapped_aoi_envelope(&geometry.geometry).unwrap();

    assert!(!envelope.crosses_antimeridian);
    assert_eq!(
        (envelope.display_west, envelope.display_east),
        (-180.0, 180.0)
    );
    assert_eq!(
        envelope.request_windows,
        vec![BBox {
            west: -180.0,
            south: -90.0,
            east: 180.0,
            north: 90.0,
            crs: "EPSG:4326".into(),
        }]
    );
}

#[test]
fn malformed_or_multi_world_geometry_fails_closed() {
    assert!(wrapped_aoi_envelope(&MultiPolygon(Vec::new())).is_none());

    let non_finite =
        polygonal_geometry(&[(f64::NAN, 0.0), (1.0, 0.0), (1.0, 1.0), (f64::NAN, 0.0)]);
    assert!(wrapped_aoi_envelope(&non_finite).is_none());

    let multi_world = polygonal_geometry(&[
        (0.0, 0.0),
        (170.0, 1.0),
        (-20.0, 2.0),
        (150.0, 3.0),
        (-40.0, 4.0),
        (0.0, 0.0),
    ]);
    assert!(wrapped_aoi_envelope(&multi_world).is_none());

    let no_closed_lift = polygonal_geometry(&[(0.0, 0.0), (120.0, 0.0), (-120.0, 1.0), (0.0, 0.0)]);
    assert!(wrapped_aoi_envelope(&no_closed_lift).is_none());
}

#[test]
fn alignment_rejects_invalid_or_incompatible_display_domains() {
    let geometry = validated_authoritative_polygonal_geojson(&fixture("hubei")).unwrap();

    assert!(align_polygonal_to_longitude_domain(&geometry.geometry, 116.0, 108.0).is_none());
    assert!(align_polygonal_to_longitude_domain(&geometry.geometry, 0.0, 361.0).is_none());
    assert!(
        align_polygonal_to_longitude_domain(&geometry.geometry, -180.0, 180.00000000000003)
            .is_none()
    );
    assert!(align_polygonal_to_longitude_domain(&geometry.geometry, f64::NAN, 116.0).is_none());
    assert!(align_polygonal_to_longitude_domain(&geometry.geometry, 109.0, 115.0).is_none());
}
