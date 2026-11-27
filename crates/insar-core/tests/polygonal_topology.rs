use insar_core::polygonal_topology::{MAX_POLYGONAL_COMPONENTS, MAX_POLYGONAL_COORDS_PER_RING};
use insar_core::{validated_authoritative_polygonal_geojson, validated_polygonal_geojson};
use serde_json::{json, Map, Value};

fn dense_polygon(position_count: usize) -> Map<String, Value> {
    assert!(position_count >= 4);
    let bottom_vertices = position_count - 3;
    let mut ring = (0..bottom_vertices)
        .map(|index| {
            let fraction = index as f64 / (bottom_vertices - 1) as f64;
            json!([113.0 + 7.0 * fraction, 36.0])
        })
        .collect::<Vec<_>>();
    ring.extend([
        json!([120.0, 43.0]),
        json!([113.0, 43.0]),
        json!([113.0, 36.0]),
    ]);
    json!({"type": "Polygon", "coordinates": [ring]})
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn authoritative_administrative_ring_is_not_rejected_by_the_asf_projection_bound() {
    // The national DataV authority reaches 4,995 positions in one ring. The
    // complete authority must remain bindable even though the independently
    // bounded ASF projection parser deliberately refuses it.
    let national_scale = dense_polygon(4_995);
    assert!(validated_authoritative_polygonal_geojson(&national_scale).is_some());
    assert!(validated_polygonal_geojson(&national_scale).is_none());

    assert!(validated_polygonal_geojson(&dense_polygon(1_549)).is_some());
    assert!(validated_polygonal_geojson(&dense_polygon(MAX_POLYGONAL_COORDS_PER_RING)).is_some());
    assert!(
        validated_polygonal_geojson(&dense_polygon(MAX_POLYGONAL_COORDS_PER_RING + 1)).is_none()
    );
}

fn disjoint_components(component_count: usize) -> Map<String, Value> {
    let components = (0..component_count)
        .map(|index| {
            let west = 100.0 + index as f64 * 0.01;
            vec![vec![
                json!([west, 20.0]),
                json!([west + 0.004, 20.0]),
                json!([west + 0.004, 20.004]),
                json!([west, 20.004]),
                json!([west, 20.0]),
            ]]
        })
        .collect::<Vec<_>>();
    json!({"type": "MultiPolygon", "coordinates": components})
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn authoritative_national_components_are_not_rejected_by_the_asf_projection_bound() {
    // The national DataV authority contains 277 provider components before
    // union. Authority admission is byte/topology bounded, not coupled to the
    // smaller remote-request projection budget.
    let national_scale = disjoint_components(277);
    assert!(validated_authoritative_polygonal_geojson(&national_scale).is_some());
    assert!(validated_polygonal_geojson(&national_scale).is_none());

    assert!(validated_polygonal_geojson(&disjoint_components(MAX_POLYGONAL_COMPONENTS)).is_some());
    assert!(
        validated_polygonal_geojson(&disjoint_components(MAX_POLYGONAL_COMPONENTS + 1)).is_none()
    );
}

#[test]
fn catalogue_query_validation_scales_linearly_on_twenty_thousand_vertex_aoi_and_rejects_degenerate_rings(
) {
    let dense_aoi = dense_polygon(20_000);
    let query: insar_core::CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l1c",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": dense_aoi,
        "cursor": null,
        "result_limit": 100,
        "filters": {"product_type": "S2MSI1C"}
    }))
    .unwrap();
    let started = std::time::Instant::now();
    for _ in 0..40 {
        query
            .validate()
            .expect("20,000-vertex administrative AOI must validate");
    }
    assert!(
        started.elapsed().as_millis() < 500,
        "40 validations of 20,000-vertex AOI took {:?}",
        started.elapsed()
    );

    let degenerate: insar_core::CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l1c",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[117.0, 31.0], [118.0, 32.0], [117.0, 31.0], [117.0, 31.0]]]
        },
        "cursor": null,
        "result_limit": 10,
        "filters": {}
    }))
    .unwrap();
    assert!(degenerate
        .validate()
        .unwrap_err()
        .contains("three distinct positions"));
}
