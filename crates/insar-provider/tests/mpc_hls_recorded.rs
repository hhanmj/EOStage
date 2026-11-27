//! Issue #3 R2 B5: real Microsoft Planetary Computer HLS v2.0 search pages
//! (recorded 2026-10-19 with the exact request the adapter builds) pass the
//! HLS adapter's request builder, item mapper and reviewed-identity checks.
//! The live TLS transport is exercised separately (see the B5 spec).
use insar_core::CatalogSearchQuery;
use insar_provider::catalog::{
    build_stac_search_request, map_stac_complete_search_response, map_stac_item,
    mpc_hls_route_profile, normalize_mpc_hls_query, validate_mpc_hls_product, MpcHlsProduct,
    MPC_HLS_PROVIDER_ID,
};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/mpc_hls/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn query(product: MpcHlsProduct, platforms: Value) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: product.logical_dataset().to_owned(),
        provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
        native_collection_id: product.collection_id().to_owned(),
        aoi_geojson: Some(
            json!({"type":"Polygon","coordinates":[[[110.0,30.0],[110.5,30.0],[110.5,30.5],[110.0,30.5],[110.0,30.0]]]})
                .as_object()
                .unwrap()
                .clone(),
        ),
        cursor: None,
        result_limit: Some(5),
        filters: json!({
            "spacecraft": platforms,
            "cloud_cover_lte": 80.0,
            "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

/// JSON equality that treats 80 and 80.0 as the same number.
fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| same(v, w)))
        }
        _ => left == right,
    }
}

#[test]
fn recorded_requests_equal_the_adapter_request_bodies() {
    for (product, platforms, name) in [
        (
            MpcHlsProduct::L30,
            json!(["landsat_8", "landsat_9"]),
            "hls2-l30.request.2026-10-19.json",
        ),
        (
            MpcHlsProduct::S30,
            json!(["sentinel_2a", "sentinel_2b"]),
            "hls2-s30.request.2026-10-19.json",
        ),
    ] {
        let (bound, normalized) = normalize_mpc_hls_query(&query(product, platforms)).unwrap();
        assert_eq!(bound, product);
        let request =
            build_stac_search_request(&mpc_hls_route_profile(), &product.binding(), &normalized)
                .unwrap();
        let body = Value::Object(request.body.unwrap());
        assert!(same(&body, &fixture(name)), "{name}: {body}");
    }
}

#[test]
fn recorded_items_pass_the_reviewed_identity_checks() {
    let route = mpc_hls_route_profile();
    for (product, name, expected) in [
        (MpcHlsProduct::L30, "hls2-l30.search.2026-10-19.json", 2),
        (MpcHlsProduct::S30, "hls2-s30.search.2026-10-19.json", 5),
    ] {
        let page = fixture(name);
        let features = page["features"].as_array().unwrap();
        assert_eq!(features.len(), expected, "{name}");
        for feature in features {
            let mapped = map_stac_item(&route, &product.binding(), feature).unwrap();
            let reviewed = validate_mpc_hls_product(product, mapped).unwrap();
            assert!(!reviewed.assets.is_empty() && reviewed.payload.is_empty());
            let platform = reviewed.properties["platform"].as_str().unwrap();
            assert!(product
                .platforms()
                .iter()
                .any(|(neutral, _)| *neutral == platform));
            let cloud = reviewed.properties["cloud_cover"].as_f64().unwrap();
            assert!((0.0..=80.0).contains(&cloud), "{cloud}");
        }
    }
}

#[test]
fn the_recorded_l30_page_maps_to_one_complete_catalogue_response() {
    let product = MpcHlsProduct::L30;
    let (_, normalized) =
        normalize_mpc_hls_query(&query(product, json!(["landsat_8", "landsat_9"]))).unwrap();
    let response = map_stac_complete_search_response(
        &mpc_hls_route_profile(),
        &product.binding(),
        &normalized,
        &fixture("hls2-l30.search.2026-10-19.json"),
    )
    .unwrap();
    assert_eq!(response.products.len(), 2);
    assert_eq!(response.collection.native_collection_id, "hls2-l30");
    assert!(response.next_cursor.is_none());
}
