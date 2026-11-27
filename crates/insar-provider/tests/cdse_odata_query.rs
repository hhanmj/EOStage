use insar_core::CatalogSearchQuery;
use insar_provider::catalog::RouteUse;
use insar_provider::cdse::query::{
    build_odata_request, build_odata_request_for_use, wrap_continuation,
};
use serde_json::json;
use url::Url;

fn valid_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "cursor": null,
        "result_limit": 2,
        "filters": {
            "product_type": "S2MSI2A",
            "cloud_cover_lte": 4.2,
            "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
        }
    }))
    .expect("valid generic catalogue query")
}

#[test]
fn sentinel2_spacecraft_filter_preserves_selection_and_cursor_identity() {
    let mut query = valid_query();
    query.filters.insert(
        "spacecraft".to_owned(),
        json!(["sentinel_2c", "sentinel_2a"]),
    );
    let request = build_odata_request_for_use(&query, RouteUse::Production).unwrap();
    let filter = url::form_urlencoded::parse(
        request
            .path_and_query()
            .split_once('?')
            .unwrap()
            .1
            .as_bytes(),
    )
    .find(|(key, _)| key == "$filter")
    .unwrap()
    .1
    .into_owned();
    assert!(filter.contains("(startswith(Name,'S2A_') or startswith(Name,'S2C_'))"));
    assert!(!filter.contains("S2B_"));
    assert_ne!(
        request.fingerprint(),
        build_odata_request_for_use(&valid_query(), RouteUse::Production)
            .unwrap()
            .fingerprint()
    );
    query.filters.insert(
        "spacecraft".to_owned(),
        json!(["sentinel_2a", "sentinel_2c"]),
    );
    assert_eq!(
        request.fingerprint(),
        build_odata_request_for_use(&query, RouteUse::Production)
            .unwrap()
            .fingerprint()
    );
    for invalid in [
        json!([]),
        json!(["sentinel_2d"]),
        json!(["sentinel_2a", "sentinel_2a"]),
        json!(null),
    ] {
        query.filters.insert("spacecraft".to_owned(), invalid);
        assert!(build_odata_request_for_use(&query, RouteUse::Production).is_err());
    }
}

fn l1c_query() -> CatalogSearchQuery {
    let mut query = valid_query();
    query.logical_dataset = "optical.sentinel-2.l1c".to_owned();
    query.filters["product_type"] = json!("S2MSI1C");
    query
}

#[test]
fn validated_sentinel2_query_builds_the_reviewed_odata_shape() {
    let request = build_odata_request(&valid_query()).expect("private facts translate");
    assert!(request.path_and_query().starts_with("Products?"));
    assert!(request
        .path_and_query()
        .contains("%24orderby=ContentDate%2FStart+asc"));
    assert!(!request.path_and_query().contains("Id+asc"));
    assert!(request.path_and_query().contains("OData.CSC.Intersects"));
    assert!(request.path_and_query().contains("%24expand=Attributes"));

    let production = build_odata_request_for_use(&valid_query(), RouteUse::Production)
        .expect("production facts translate");
    assert!(production.path_and_query().contains("%24expand=Attributes"));
    assert!(!production.path_and_query().contains("Assets"));
    let production_options = url::form_urlencoded::parse(
        production
            .path_and_query()
            .split_once('?')
            .expect("request has query")
            .1
            .as_bytes(),
    )
    .collect::<Vec<_>>();
    assert_eq!(
        production_options
            .iter()
            .filter(|(key, _)| key.as_ref() == "$expand")
            .map(|(_, value)| value.as_ref())
            .collect::<Vec<_>>(),
        ["Attributes"]
    );
    assert!(production_options.iter().all(|(key, _)| {
        matches!(
            key.as_ref(),
            "$count" | "$expand" | "$filter" | "$orderby" | "$top"
        )
    }));
    assert_eq!(request.fingerprint(), production.fingerprint());
}

#[test]
fn sentinel2_all_time_and_long_range_queries_keep_bounded_result_intent() {
    let mut all_time = valid_query();
    all_time.filters.remove("datetime");
    let request = build_odata_request_for_use(&all_time, RouteUse::Production)
        .expect("all-time Sentinel-2 query is valid");
    let filter = url::form_urlencoded::parse(
        request
            .path_and_query()
            .split_once('?')
            .unwrap()
            .1
            .as_bytes(),
    )
    .find(|(key, _)| key == "$filter")
    .unwrap()
    .1
    .into_owned();
    assert!(!filter.contains("ContentDate/Start ge"));
    assert!(!filter.contains("ContentDate/Start lt"));
    assert_ne!(
        request.fingerprint(),
        build_odata_request_for_use(&valid_query(), RouteUse::Production)
            .unwrap()
            .fingerprint()
    );

    let mut long_range = valid_query();
    long_range.filters["datetime"] = json!({
        "start": "2016-01-01T00:00:00Z",
        "end": "2026-01-01T00:00:00Z"
    });
    assert!(build_odata_request_for_use(&long_range, RouteUse::Production).is_ok());
}

#[test]
fn sentinel2_l1c_offline_query_uses_its_own_product_type_and_cursor_scope() {
    let l1c = build_odata_request(&l1c_query()).expect("L1C translates only offline");
    let filter = url::form_urlencoded::parse(
        l1c.path_and_query()
            .split_once('?')
            .expect("request has query")
            .1
            .as_bytes(),
    )
    .find(|(key, _)| key == "$filter")
    .expect("request has a filter")
    .1
    .into_owned();
    assert!(filter.contains("S2MSI1C"));
    assert!(!filter.contains("S2MSI2A"));
    assert_ne!(
        l1c.fingerprint(),
        build_odata_request(&valid_query()).unwrap().fingerprint()
    );
    assert!(
        build_odata_request_for_use(&l1c_query(), RouteUse::Production).is_ok(),
        "the L1C route is production-enabled"
    );

    let mut wrong_product_type = l1c_query();
    wrong_product_type.filters["product_type"] = json!("S2MSI2A");
    assert!(build_odata_request(&wrong_product_type).is_err());
}

#[test]
fn typed_filter_facts_reject_open_invalid_or_cross_scope_values() {
    for (field, value) in [
        ("product_type", json!("S2MSI1C")),
        ("cloud_cover_lte", json!(101)),
        (
            "datetime",
            json!({"start":"2026-01-01", "end":"2026-01-02T00:00:00Z"}),
        ),
    ] {
        let mut query = valid_query();
        query.filters.insert(field.to_string(), value);
        assert!(build_odata_request(&query).is_err(), "{field}");
    }
    let mut query = valid_query();
    query.filters.remove("cloud_cover_lte");
    assert!(
        build_odata_request(&query).is_ok(),
        "cloud cover is optional"
    );

    let first = build_odata_request_for_use(&valid_query(), RouteUse::Production).unwrap();
    let next = Url::parse("http://127.0.0.1:38081/odata/v1/Products?$count=True&$expand=Attributes&$filter=x&$orderby=ContentDate/Start%20asc&$skip=100&$top=100").unwrap();
    let mut replay = valid_query();
    replay.cursor = Some(wrap_continuation(first.fingerprint(), &next).unwrap());
    replay
        .aoi_geojson
        .as_mut()
        .expect("fixture query includes AOI")["coordinates"] =
        json!([[[109.0, 30.0], [111.0, 30.0], [111.0, 31.0], [109.0, 30.0]]]);
    assert!(
        build_odata_request_for_use(&replay, RouteUse::Production).is_err(),
        "cursor binds exact AOI topology"
    );

    for mutation in ["time", "cloud", "result_limit", "provider", "collection"] {
        let mut replay = valid_query();
        replay.cursor = Some(wrap_continuation(first.fingerprint(), &next).unwrap());
        match mutation {
            "time" => replay.filters["datetime"]["end"] = json!("2026-01-03T00:00:00Z"),
            "cloud" => replay.filters["cloud_cover_lte"] = json!(5.0),
            "result_limit" => replay.result_limit = Some(3),
            "provider" => replay.provider_id = "other.provider".to_string(),
            "collection" => replay.native_collection_id = "OTHER".to_string(),
            _ => unreachable!(),
        }
        assert!(
            build_odata_request_for_use(&replay, RouteUse::Production).is_err(),
            "{mutation}"
        );
    }
    let mut larger_result = valid_query();
    larger_result.result_limit = Some(617);
    let request = build_odata_request(&larger_result)
        .expect("result intent does not redefine the CDSE transport page size");
    assert_eq!(request.page_size(), 100);
    assert!(request.path_and_query().contains("%24top=100"));
}

#[test]
fn fractional_seconds_remain_in_the_query_and_cursor_fingerprint() {
    let mut with_fraction = valid_query();
    with_fraction.filters["datetime"]["start"] = json!("2026-01-01T00:00:00.123400Z");
    with_fraction.filters["datetime"]["end"] = json!("2026-01-02T00:00:00.000001Z");
    let fractional = build_odata_request(&with_fraction).expect("fractional UTC query is valid");
    let filter = url::form_urlencoded::parse(
        fractional
            .path_and_query()
            .split_once('?')
            .unwrap()
            .1
            .as_bytes(),
    )
    .find(|(key, _)| key == "$filter")
    .unwrap()
    .1
    .into_owned();
    assert!(filter.contains("2026-01-01T00:00:00.123400Z"));
    assert!(filter.contains("2026-01-02T00:00:00.000001Z"));
    assert_ne!(
        fractional.fingerprint(),
        build_odata_request(&valid_query()).unwrap().fingerprint(),
        "the cursor binding keeps subsecond query truth"
    );
}

#[test]
fn continuation_size_limits_fail_before_url_or_cursor_replay() {
    let request = build_odata_request(&valid_query()).unwrap();
    let oversized_link = Url::parse(&format!(
        "http://127.0.0.1:38081/odata/v1/Products?{}",
        "x".repeat(8 * 1024)
    ))
    .unwrap();
    assert!(wrap_continuation(request.fingerprint(), &oversized_link).is_err());

    let mut replay = valid_query();
    replay.cursor = Some("x".repeat(24 * 1024 + 1));
    assert!(build_odata_request(&replay).is_err());
}

#[test]
fn malformed_cursor_text_is_total_and_rejected_without_a_panic() {
    let request = build_odata_request(&valid_query()).unwrap();
    for encoded in ["€a", "a", "AA", "ag", ""] {
        let mut query = valid_query();
        query.cursor = Some(format!("cdse-odata.v1:{}:{encoded}", request.fingerprint()));
        let result = std::panic::catch_unwind(|| build_odata_request(&query));
        assert!(result.is_ok(), "{encoded:?} must not panic");
        assert!(result.unwrap().is_err(), "{encoded:?} must be rejected");
    }

    for fingerprint in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        let mut query = valid_query();
        query.cursor = Some(format!("cdse-odata.v1:{fingerprint}:61"));
        assert!(build_odata_request(&query).is_err(), "{fingerprint}");
    }
}

#[test]
fn fingerprint_binds_effective_spatial_clause_not_feature_metadata() {
    let polygon = valid_query();
    let polygon_request = build_odata_request(&polygon).unwrap();

    let mut feature = valid_query();
    feature.aoi_geojson = serde_json::from_value(json!({
        "type":"Feature",
        "properties":{"label":"first","ordinary":true},
        "geometry": polygon.aoi_geojson,
    }))
    .unwrap();
    let feature_request = build_odata_request(&feature).unwrap();
    assert_eq!(
        polygon_request.path_and_query(),
        feature_request.path_and_query()
    );
    assert_eq!(polygon_request.fingerprint(), feature_request.fingerprint());

    feature.aoi_geojson.as_mut().expect("feature includes AOI")["properties"]["label"] =
        json!("second");
    assert_eq!(
        feature_request.fingerprint(),
        build_odata_request(&feature).unwrap().fingerprint(),
        "ordinary Feature properties do not affect cursor scope"
    );

    let mut changed_coordinate = feature.clone();
    changed_coordinate
        .aoi_geojson
        .as_mut()
        .expect("feature includes AOI")["geometry"]["coordinates"][0][1][0] = json!(111.5);
    assert_ne!(
        feature_request.fingerprint(),
        build_odata_request(&changed_coordinate)
            .unwrap()
            .fingerprint()
    );
    let mut multipolygon = valid_query();
    multipolygon.aoi_geojson = serde_json::from_value(json!({
        "type":"MultiPolygon",
        "coordinates":[
            [[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]],
            [[[112.0,30.0],[113.0,30.0],[113.0,31.0],[112.0,30.0]]]
        ]
    }))
    .unwrap();
    let ordered = build_odata_request(&multipolygon).unwrap();
    multipolygon
        .aoi_geojson
        .as_mut()
        .expect("multipolygon includes AOI")["coordinates"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_ne!(
        ordered.fingerprint(),
        build_odata_request(&multipolygon).unwrap().fingerprint(),
        "polygon ordering is part of the emitted OData spatial clause"
    );
    let mut changed_filter = feature.clone();
    changed_filter.filters["cloud_cover_lte"] = json!(4.1);
    assert_ne!(
        feature_request.fingerprint(),
        build_odata_request(&changed_filter).unwrap().fingerprint()
    );
    let mut changed_result_limit = feature;
    changed_result_limit.result_limit = Some(3);
    assert_ne!(
        feature_request.fingerprint(),
        build_odata_request(&changed_result_limit)
            .unwrap()
            .fingerprint()
    );
}

#[test]
fn multipolygon_uses_or_of_polygon_intersects_without_bbox_substitution() {
    let mut query = valid_query();
    query.aoi_geojson = serde_json::from_value(json!({
        "type": "MultiPolygon",
        "coordinates": [
            [[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]],
            [[[112.0,30.0],[113.0,30.0],[113.0,31.0],[112.0,30.0]]]
        ]
    }))
    .unwrap();
    let request = build_odata_request(&query).unwrap();
    let decoded = url::form_urlencoded::parse(
        request
            .path_and_query()
            .split_once('?')
            .unwrap()
            .1
            .as_bytes(),
    )
    .find(|(key, _)| key == "$filter")
    .unwrap()
    .1
    .into_owned();
    assert_eq!(decoded.matches("OData.CSC.Intersects").count(), 2);
    assert!(decoded.contains(" or "));
    assert!(!decoded.contains("MULTIPOLYGON"));
}
