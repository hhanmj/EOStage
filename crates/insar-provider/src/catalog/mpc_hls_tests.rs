use serde_json::json;

use super::*;
use crate::catalog::{build_stac_search_request, map_stac_item};

fn query(product: MpcHlsProduct, filters: Value) -> CatalogSearchQuery {
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
        result_limit: Some(100),
        filters: filters.as_object().unwrap().clone(),
    }
}

/// Shape of a real Microsoft HLS v2.0 item (recorded 2026-10-19), trimmed.
fn item(product: MpcHlsProduct, id: &str, platform: &str, cloud: Value) -> Value {
    json!({
        "type": "Feature",
        "stac_version": "1.0.0",
        "stac_extensions": [
            "https://stac-extensions.github.io/eo/v1.1.0/schema.json",
            "https://stac-extensions.github.io/projection/v1.1.0/schema.json"
        ],
        "id": id,
        "collection": product.collection_id(),
        "geometry": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.1,30.0],[111.1,31.0],[110.0,31.0],[110.0,30.0]]]},
        "bbox": [110.0, 30.0, 111.1, 31.0],
        "properties": {
            "datetime": "2024-06-25T03:07:56.017810Z",
            "platform": platform,
            "instruments": ["oli"],
            "eo:cloud_cover": cloud,
            "proj:epsg": 32649,
            "sci:doi": "10.5067/HLS/HLSL30.002"
        },
        "assets": {
            "B04": {
                "href": "https://hls2euwest.blob.core.windows.net/hls2/L30/49/R/DP/2024/06/25/B04.tif",
                "type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "roles": ["data"]
            },
            "Fmask": {
                "href": "https://hls2euwest.blob.core.windows.net/hls2/L30/49/R/DP/2024/06/25/Fmask.tif",
                "type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "roles": ["data"]
            }
        },
        "links": [{"rel": "self", "href": "https://planetarycomputer.microsoft.com/api/stac/v1/collections/hls2-l30/items/x"}]
    })
}

#[test]
fn bindings_are_valid_search_only_and_source_exact() {
    let route = mpc_hls_route_profile();
    for product in MpcHlsProduct::ALL {
        let binding = product.binding();
        binding.validate(&route).unwrap();
        assert!(!binding.asset_role_bindings.is_empty());
        assert_eq!(
            MpcHlsProduct::for_source(
                product.logical_dataset(),
                MPC_HLS_PROVIDER_ID,
                product.collection_id()
            ),
            Some(product)
        );
    }
    assert_eq!(
        MpcHlsProduct::for_source(
            MPC_HLS_L30_LOGICAL_DATASET,
            MPC_HLS_PROVIDER_ID,
            MPC_HLS_S30_COLLECTION_ID
        ),
        None
    );
    assert_eq!(
        MpcHlsProduct::for_source(
            MPC_HLS_L30_LOGICAL_DATASET,
            "element84.earth-search",
            MPC_HLS_L30_COLLECTION_ID
        ),
        None
    );
}

#[test]
fn queries_translate_platforms_and_reject_undeclared_or_foreign_values() {
    let (product, normalized) = normalize_query(&query(
        MpcHlsProduct::L30,
        json!({
            "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"},
            "cloud_cover_lte": 20,
            "spacecraft": ["landsat_9", "landsat_8"]
        }),
    ))
    .unwrap();
    assert_eq!(product, MpcHlsProduct::L30);
    assert_eq!(
        normalized.filters.get("spacecraft"),
        Some(&json!(["landsat-9", "landsat-8"]))
    );
    let request =
        build_stac_search_request(&mpc_hls_route_profile(), &product.binding(), &normalized)
            .unwrap();
    let body = request.body.unwrap();
    assert_eq!(body.get("collections"), Some(&json!(["hls2-l30"])));
    assert_eq!(
        body.get("query"),
        Some(
            &json!({"eo:cloud_cover": {"lte": 20.0}, "platform": {"in": ["landsat-8", "landsat-9"]}})
        )
    );
    assert!(body.contains_key("intersects"));
    assert!(body.contains_key("datetime"));

    for filters in [
        json!({"spacecraft": ["sentinel_2a"]}),
        json!({"spacecraft": ["landsat-8"]}),
        json!({"spacecraft": []}),
        json!({"spacecraft": ["landsat_8", "landsat_8"]}),
        json!({"cloud_cover_lte": 101}),
        json!({"cloud_cover_lte": "20"}),
        json!({"product_type": "L30"}),
        json!({"eo:cloud_cover": 10}),
    ] {
        assert!(
            normalize_query(&query(MpcHlsProduct::L30, filters.clone())).is_err(),
            "{filters}"
        );
    }
    let (s30, normalized) = normalize_query(&query(
        MpcHlsProduct::S30,
        json!({"spacecraft": ["sentinel_2c"]}),
    ))
    .unwrap();
    assert_eq!(s30, MpcHlsProduct::S30);
    assert_eq!(
        normalized.filters.get("spacecraft"),
        Some(&json!(["sentinel-2c"]))
    );

    let mut paged = query(MpcHlsProduct::L30, json!({}));
    paged.cursor = Some("opaque".to_owned());
    assert!(normalize_query(&paged).is_err());
    let mut unbounded = query(MpcHlsProduct::L30, json!({}));
    unbounded.result_limit = None;
    assert!(normalize_query(&unbounded).is_err());
    let mut foreign = query(MpcHlsProduct::L30, json!({}));
    foreign.native_collection_id = "landsat-c2-l2".to_owned();
    assert!(normalize_query(&foreign).is_err());
}

#[test]
fn mapped_items_keep_only_reviewed_scene_facts() {
    let route = mpc_hls_route_profile();
    let product = MpcHlsProduct::L30;
    let mapped = map_stac_item(
        &route,
        &product.binding(),
        &item(
            product,
            "HLS.L30.T49RDP.2024177T030756.v2.0",
            "landsat-8",
            json!(79),
        ),
    )
    .unwrap();
    let reviewed = validate_product(product, mapped).unwrap();
    assert_eq!(reviewed.product_id, "HLS.L30.T49RDP.2024177T030756.v2.0");
    assert_eq!(reviewed.display_name, reviewed.product_id);
    assert_eq!(reviewed.source_kind, DataSourceKind::Landsat);
    assert_eq!(reviewed.product_kind, ProductKind::OpticalScene);
    assert!(!reviewed.assets.is_empty());
    assert!(reviewed.payload.is_empty());
    assert_eq!(
        Value::Object(reviewed.properties.clone()),
        json!({"platform": "landsat_8", "cloud_cover": 79.0})
    );
    let serialized = serde_json::to_string(&reviewed).unwrap();
    assert!(serialized.contains("blob.core.windows.net"));
    assert!(!serialized.contains("stac_properties"));
}

#[test]
fn mismatched_items_are_rejected() {
    let route = mpc_hls_route_profile();
    let l30 = MpcHlsProduct::L30;
    let reject = |product: MpcHlsProduct, value: Value| {
        map_stac_item(&route, &product.binding(), &value)
            .and_then(|mapped| validate_product(product, mapped))
            .is_err()
    };
    // An S30 identifier inside the L30 collection.
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.S30.T49RDP.2024177T030756.v2.0",
            "landsat-8",
            json!(10)
        )
    ));
    // A Sentinel-2 platform in the Landsat product.
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.L30.T49RDP.2024177T030756.v2.0",
            "sentinel-2a",
            json!(10)
        )
    ));
    // Cloud cover outside 0..=100 or missing.
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.L30.T49RDP.2024177T030756.v2.0",
            "landsat-8",
            json!(120)
        )
    ));
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.L30.T49RDP.2024177T030756.v2.0",
            "landsat-8",
            Value::Null
        )
    ));
    // Version 1.5 or a malformed tile.
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.L30.T49RDP.2024177T030756.v1.5",
            "landsat-8",
            json!(10)
        )
    ));
    assert!(reject(
        l30,
        item(
            l30,
            "HLS.L30.49RDP.2024177T030756.v2.0",
            "landsat-8",
            json!(10)
        )
    ));
    // The collection itself must match the binding.
    let mut foreign = item(
        l30,
        "HLS.L30.T49RDP.2024177T030756.v2.0",
        "landsat-8",
        json!(10),
    );
    foreign["collection"] = json!("hls2-s30");
    assert!(reject(l30, foreign));
    // Review M2: the scene id's day and time must name the sensed scene.
    let mut another_day = item(
        l30,
        "HLS.L30.T49RDP.2024176T030756.v2.0",
        "landsat-8",
        json!(10),
    );
    assert!(reject(l30, another_day.clone()));
    another_day["properties"]["datetime"] = json!("2024-06-24T03:07:56Z");
    assert!(!reject(l30, another_day));
}

#[test]
fn scene_time_matches_the_sensed_time_within_the_datatake_window() {
    // L30: the id names the acquisition itself.
    assert!(scene_time_matches(
        "HLS.L30.T49RDP.2024177T030756.v2.0",
        "2024-06-25T03:07:56.017810Z"
    ));
    // S30: the id names the datatake start; tiles are sensed minutes later
    // (recorded: 03:15:21 -> 03:29:46).
    assert!(scene_time_matches(
        "HLS.S30.T49RCP.2024175T031521.v2.0",
        "2024-06-23T03:29:46.751600Z"
    ));
    assert!(!scene_time_matches(
        "HLS.S30.T49RCP.2024175T031521.v2.0",
        "2024-06-23T06:00:00Z"
    ));
    assert!(!scene_time_matches(
        "HLS.S30.T49RCP.2024175T031521.v2.0",
        "2024-06-23T03:00:00Z"
    ));
    assert!(!scene_time_matches(
        "HLS.L30.T49RDP.2024177T030756.v2.0",
        "not a time"
    ));
    // L30 matches to the second (400 sampled items: 0.001-0.989 s); S30 ids
    // name the datatake start (400 sampled items: +176..+696 s).
    assert!(!scene_time_matches(
        "HLS.L30.T49RDP.2024177T030756.v2.0",
        "2024-06-25T03:07:57.2Z"
    ));
    assert!(!scene_time_matches(
        "HLS.L30.T49RDP.2024177T030756.v2.0",
        "2024-06-25T03:07:55.9Z"
    ));
    assert!(!valid_scene_id(
        MpcHlsProduct::L30,
        "HLS.L30.T49RDP.+024177T030756.v2.0"
    ));
}

#[test]
fn scene_identifier_grammar_is_exact() {
    assert!(valid_scene_id(
        MpcHlsProduct::S30,
        "HLS.S30.T49RCP.2024175T031521.v2.0"
    ));
    assert!(!valid_scene_id(
        MpcHlsProduct::S30,
        "HLS.S30.T49RCP.2024175T031521.v2.0.tif"
    ));
    assert!(!valid_scene_id(
        MpcHlsProduct::S30,
        "HLS.S30.T49rcp.2024175T031521.v2.0"
    ));
    assert!(!valid_scene_id(
        MpcHlsProduct::S30,
        "HLS.S30.T49RCP.2024175031521.v2.0"
    ));
    assert!(!valid_scene_id(
        MpcHlsProduct::L30,
        "HLS.S30.T49RCP.2024175T031521.v2.0"
    ));
    // Review M2: a real day of year and clock time.
    for invalid in [
        "HLS.S30.T49RCP.2024000T031521.v2.0",
        "HLS.S30.T49RCP.2023366T031521.v2.0",
        "HLS.S30.T49RCP.2024367T031521.v2.0",
        "HLS.S30.T49RCP.2024175T251521.v2.0",
        "HLS.S30.T49RCP.2024175T036021.v2.0",
    ] {
        assert!(!valid_scene_id(MpcHlsProduct::S30, invalid), "{invalid}");
    }
    assert!(valid_scene_id(
        MpcHlsProduct::S30,
        "HLS.S30.T49RCP.2024366T031521.v2.0"
    ));
}

// --- Oversized (China-like) AOI: envelope request, exact admission ---------

use super::super::spatial::dense_disjoint_china_like_aoi_for_test;
use super::super::stac_test_server::JsonPageServer;

fn l30_item_at(tile: &str, west: f64, south: f64) -> Value {
    let mut feature = item(
        MpcHlsProduct::L30,
        &format!("HLS.L30.{tile}.2024177T030756.v2.0"),
        "landsat-8",
        json!(12.5),
    );
    feature["geometry"] = json!({"type":"Polygon","coordinates":[[
        [west, south], [west + 1.0, south], [west + 1.0, south + 1.0],
        [west, south + 1.0], [west, south]
    ]]});
    feature["bbox"] = json!([west, south, west + 1.0, south + 1.0]);
    feature
}

fn hls_page(features: Vec<Value>, next: bool) -> Value {
    let links = if next {
        vec![json!({"rel": "next", "href": "/api/stac/v1/search?token=next"})]
    } else {
        Vec::new()
    };
    json!({"type": "FeatureCollection", "numberReturned": features.len(),
        "numberMatched": 999, "features": features, "links": links})
}

fn oversized_query(limit: u64, dated: bool) -> CatalogSearchQuery {
    let filters = if dated {
        json!({"datetime":{"start":"2024-06-01T00:00:00Z","end":"2024-07-01T00:00:00Z"}})
    } else {
        json!({})
    };
    let mut query = query(MpcHlsProduct::L30, filters);
    query.aoi_geojson = Some(dense_disjoint_china_like_aoi_for_test());
    query.result_limit = Some(limit);
    query
}

fn loopback_hls(server: &JsonPageServer) -> MpcHlsCatalogClient {
    MpcHlsCatalogClient::new(ProviderNetworkPolicy::direct())
        .unwrap()
        .loopback_for_test(
            &server.origin,
            StacTransportLimits {
                request_timeout: Duration::from_secs(10),
                overall_timeout: Duration::from_secs(30),
                max_pages: 128,
                max_records: insar_core::SEARCH_SAFETY_CAP as usize,
                max_page_bytes: 4 * 1024 * 1024,
                max_total_bytes: 16 * 1024 * 1024,
            },
        )
        .unwrap()
}

#[test]
fn oversized_aoi_sends_an_envelope_and_never_admits_scenes_outside_the_real_aoi() {
    let (_, normalized) = normalize_query(&oversized_query(2, false)).unwrap();
    let request_ring = normalized.aoi_geojson.as_ref().unwrap()["coordinates"][0]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(request_ring, 5, "the oversized AOI is sent as its envelope");

    // The gap scene lies inside the envelope but outside both components; it
    // must neither be returned nor fill the cap, so the next page is read.
    let server = JsonPageServer::new(vec![
        hls_page(
            vec![
                l30_item_at("T49TAA", 108.0, 43.0),
                l30_item_at("T47SAA", 98.0, 38.0),
            ],
            true,
        ),
        hls_page(vec![l30_item_at("T51UAA", 121.0, 49.0)], false),
    ]);
    let response = loopback_hls(&server)
        .search_catalog(&oversized_query(2, false), &|| false)
        .unwrap();
    assert_eq!(
        server.count(),
        2,
        "the rejected scene did not count toward the cap"
    );
    let ids = response
        .products
        .iter()
        .map(|product| product.product_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "HLS.L30.T47SAA.2024177T030756.v2.0",
            "HLS.L30.T51UAA.2024177T030756.v2.0"
        ]
    );
    assert_eq!(response.stats.fetched_record_count, 3);
    assert_eq!(response.stats.exact_matched_count, 2);
    assert_eq!(response.stats.returned_count, 2);
    // The server total describes the envelope, not the AOI.
    assert_eq!(response.stats.matched_count, None);
    assert!(response.stats.remote_exhausted);
}

#[test]
fn oversized_aoi_streaming_flushes_only_exact_hits_and_fills_the_cap_with_them() {
    let server = JsonPageServer::new(vec![
        hls_page(vec![l30_item_at("T49TAA", 108.0, 43.0)], true),
        hls_page(
            vec![
                l30_item_at("T47SAA", 98.0, 38.0),
                l30_item_at("T49TAB", 109.0, 44.0),
            ],
            true,
        ),
    ]);
    let mut streamed = Vec::new();
    let (_, stats) = loopback_hls(&server)
        .search_catalog_streaming(&oversized_query(1, true), &|| false, &mut |page| {
            streamed.extend(page.iter().map(|product| product.product_id.clone()));
            Ok(())
        })
        .unwrap();
    assert_eq!(server.count(), 2);
    assert_eq!(streamed, ["HLS.L30.T47SAA.2024177T030756.v2.0"]);
    assert_eq!(stats.returned_count, 1);
    assert_eq!(stats.exact_matched_count, 1);
    assert_eq!(stats.fetched_record_count, 3);
    assert!(!stats.remote_exhausted);
}
