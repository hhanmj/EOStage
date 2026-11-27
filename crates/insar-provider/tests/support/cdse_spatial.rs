use super::*;

fn ring(x: f64, y: f64, radius: f64, vertices: usize) -> Vec<Value> {
    let mut points = (0..vertices)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / vertices as f64;
            json!([x + radius * angle.cos(), y + radius * angle.sin()])
        })
        .collect::<Vec<_>>();
    points.push(points[0].clone());
    points
}

fn bounds(west: f64, south: f64, east: f64, north: f64) -> Value {
    json!({"type":"Polygon", "coordinates":[[
        [west,south],[east,south],[east,north],[west,north],[west,south]
    ]]})
}

fn complex_query(vertices: usize) -> CatalogSearchQuery {
    let mut result = query(None);
    result.aoi_geojson = json!({"type":"Polygon", "coordinates":[
        ring(110.0, 30.0, 1.0, vertices),
        [[109.8,29.8],[109.8,30.2],[110.2,30.2],[110.2,29.8],[109.8,29.8]]
    ]})
    .as_object()
    .cloned();
    result
}

fn located(index: u64, x: f64, y: f64) -> Value {
    let mut record = valid_record(index);
    record["GeoFootprint"] = bounds(x, y, x + 0.05, y + 0.05);
    record
}

fn spatial_next(source: &CatalogSearchQuery, skip: u64) -> String {
    let mut part = source.clone();
    part.aoi_geojson = bounds(109.0, 29.0, 111.0, 31.0).as_object().cloned();
    part.result_limit = Some(100);
    let request = build_odata_request(&part).unwrap();
    format!(
        "http://127.0.0.1:38081/odata/v1/{}&%24skip={skip}",
        request.path_and_query()
    )
}

#[test]
fn oversized_cdse_aoi_fills_target_after_exact_filter_and_pagination_deduplication() {
    let query = complex_query(1024);
    let before = query.clone();
    let error = build_odata_request(&query).unwrap_err();
    assert_eq!(error.code, "CDSE_ODATA_AOI_TOO_COMPLEX");
    assert_eq!(error.debug_context["reason"], "encoded_query");
    let accepted = located(3, 110.5, 30.1);
    let server = ScriptedServer::start(vec![
        (
            200,
            vec![],
            page_body(
                vec![
                    located(1, 110.85, 30.85),
                    located(2, 110.0, 30.0),
                    accepted.clone(),
                ],
                Some(5),
                Some(spatial_next(&query, 3)),
            ),
            Duration::ZERO,
        ),
        (
            200,
            vec![],
            page_body(vec![accepted, located(4, 109.4, 30.1)], Some(5), None),
            Duration::ZERO,
        ),
    ]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(3),
    )
    .unwrap();
    let response = client
        .search_all(&query, &|| false)
        .expect("large valid AOI should use compact transport, not reject the user boundary");
    assert_eq!(
        query, before,
        "transport rectangles must not replace source AOI"
    );
    assert_eq!(response.products.len(), 2);
    assert!(response
        .products
        .iter()
        .all(|p| p.product_id.ends_with("000000000003") || p.product_id.ends_with("000000000004")));
    assert_eq!(response.stats.request_count, 2);
    assert_eq!(response.stats.transport_part_count, 1);
    assert_eq!(response.stats.fetched_record_count, 5);
    assert_eq!(response.stats.mapped_record_count, 5);
    assert_eq!(response.stats.unique_provider_product_count, 4);
    assert_eq!(response.stats.exact_duplicate_record_count, 1);
    assert_eq!(response.stats.exact_matched_count, 2);
    assert_eq!(
        response.stats.matched_count, None,
        "rectangle counts are not AOI counts"
    );
    assert!(response.stats.remote_exhausted && response.stats.working_set_satisfied);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|line| line.len() < 2048));
}

#[test]
fn oversized_cdse_parts_deduplicate_without_admitting_the_gap_or_truncating_all() {
    let mut query = query(None);
    query.result_limit = None;
    query.aoi_geojson = json!({"type":"MultiPolygon","coordinates":[
        [ring(98.0,40.0,1.0,1024)], [ring(124.0,50.0,1.0,1024)]
    ]})
    .as_object()
    .cloned();
    let body = page_body(
        vec![
            located(1, 98.0, 40.0),
            located(2, 124.0, 50.0),
            located(3, 110.0, 45.0),
        ],
        Some(3),
        None,
    );
    let server = ScriptedServer::start(vec![(200, vec![], body, Duration::ZERO); 8]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(5),
    )
    .unwrap();
    let response = client.search_all(&query, &|| false).unwrap();
    let parts = response.stats.transport_part_count;
    assert!((2..=8).contains(&parts));
    assert_eq!(response.products.len(), 2);
    assert_eq!(response.stats.requested_limit, None);
    assert_eq!(response.stats.fetched_record_count, parts * 3);
    assert_eq!(response.stats.exact_duplicate_record_count, (parts - 1) * 3);
    assert_eq!(response.stats.unique_provider_product_count, 3);
    assert!(response.stats.remote_exhausted);
    assert_eq!(server.requests.lock().unwrap().len() as u64, parts);
}

#[test]
fn oversized_cdse_invalid_aoi_or_cancel_does_not_make_a_rectangle_request() {
    let server = ScriptedServer::start(vec![]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(1),
    )
    .unwrap();
    let query = complex_query(5000);
    assert_eq!(
        build_odata_request(&query).unwrap_err().debug_context["reason"],
        "positions"
    );
    assert_eq!(
        client.search_all(&query, &|| true).unwrap_err().code,
        "CDSE_ODATA_CANCELLED"
    );
    let mut invalid = query;
    invalid.aoi_geojson.as_mut().unwrap()["coordinates"][0][4900] = json!([200.0, 30.0]);
    assert!(client.search_all(&invalid, &|| false).is_err());
    assert!(server.requests.lock().unwrap().is_empty());
}

#[test]
fn oversized_cdse_missing_next_page_cannot_publish_an_underfilled_aoi() {
    let query = complex_query(1024);
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![located(1, 110.0, 30.0)], Some(100), None),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        client.search_all(&query, &|| false).unwrap_err().code,
        "CDSE_ODATA_PAGE_INVALID"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn oversized_cdse_rejects_conflicting_tail_and_unknown_footprint_before_publication() {
    for missing in [false, true] {
        let mut source = complex_query(1024);
        source.result_limit = Some(1);
        let first = located(1, 110.5, 30.1);
        let mut bad = first.clone();
        if missing {
            bad["GeoFootprint"] = Value::Null;
        } else {
            bad["ContentLength"] = json!(2);
        }
        let server = ScriptedServer::start(vec![(
            200,
            vec![],
            page_body(vec![first, bad], Some(2), None),
            Duration::ZERO,
        )]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(2),
        )
        .unwrap();
        let error = client.search_all(&source, &|| false).unwrap_err();
        assert_eq!(
            error.code,
            if missing {
                "CDSE_ODATA_ITEM_INVALID"
            } else {
                "CDSE_ODATA_UUID_CONFLICT"
            }
        );
    }
}

#[test]
fn oversized_cdse_does_not_replan_remote_errors_or_untrusted_continuations() {
    for status in [401, 429, 503] {
        let server = ScriptedServer::start(vec![(status, vec![], String::new(), Duration::ZERO)]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(client.search_all(&complex_query(1024), &|| false).is_err());
        assert_eq!(
            server.requests.lock().unwrap().len(),
            1,
            "no second provider or unbounded retry"
        );
    }
    let source = complex_query(1024);
    let foreign = spatial_next(&source, 1).replace("127.0.0.1:38081", "catalogue.invalid");
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![located(1, 110.5, 30.1)], Some(3), Some(foreign)),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(
        client.search_all(&source, &|| false).unwrap_err().code,
        "CDSE_ODATA_CURSOR_UNTRUSTED"
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[test]
fn oversized_cdse_finite_target_is_not_remote_exhaustion_and_empty_results_are_honest() {
    let mut source = complex_query(5000);
    source.result_limit = Some(1);
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![located(1, 110.5, 30.1)], Some(200), None),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(5),
    )
    .unwrap();
    let response = client.search_all(&source, &|| false).unwrap();
    assert_eq!(response.products.len(), 1);
    assert!(response.stats.working_set_satisfied && !response.stats.remote_exhausted);
    let server = ScriptedServer::start(vec![(
        200,
        vec![],
        page_body(vec![located(1, 110.0, 30.0)], Some(1), None),
        Duration::ZERO,
    )]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(5),
    )
    .unwrap();
    let response = client.search_all(&source, &|| false).unwrap();
    assert!(
        response.products.is_empty()
            && response.stats.remote_exhausted
            && response.stats.working_set_satisfied
    );
}

#[test]
fn oversized_cdse_polygon_and_ring_representation_limits_use_validated_compact_requests() {
    for many_rings in [false, true] {
        let mut source = query(None);
        let geometry = if many_rings {
            let mut rings = vec![bounds(109.0, 29.0, 111.0, 31.0)["coordinates"][0].clone()];
            for i in 0..128 {
                let x = 109.1 + (i % 16) as f64 * 0.1;
                let y = 29.1 + (i / 16) as f64 * 0.1;
                rings.push(json!([
                    [x, y],
                    [x, y + 0.04],
                    [x + 0.04, y + 0.04],
                    [x + 0.04, y],
                    [x, y]
                ]));
            }
            json!({"type":"Polygon","coordinates":rings})
        } else {
            let polygons = (0..33)
                .map(|i| {
                    let x = 109.0 + (i % 11) as f64 * 0.1;
                    let y = 29.0 + (i / 11) as f64 * 0.1;
                    bounds(x, y, x + 0.05, y + 0.05)["coordinates"].clone()
                })
                .collect::<Vec<_>>();
            json!({"type":"MultiPolygon","coordinates":polygons})
        };
        source.aoi_geojson = geometry.as_object().cloned();
        let error = build_odata_request(&source).unwrap_err();
        assert_eq!(
            error.debug_context["reason"],
            if many_rings { "rings" } else { "polygons" }
        );
        let server = ScriptedServer::start(vec![(
            200,
            vec![],
            page_body(vec![], Some(0), None),
            Duration::ZERO,
        )]);
        let client = LoopbackODataClient::new(
            &format!("{}/odata/v1/", server.origin),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(client
            .search_all(&source, &|| false)
            .unwrap()
            .products
            .is_empty());
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn oversized_cdse_five_thousand_vertex_aoi_handles_one_hundred_scenes_within_one_budget() {
    let mut source = complex_query(5000);
    source.result_limit = Some(100);
    let records = (0..100)
        .map(|index| {
            located(
                index + 1,
                110.35 + (index % 10) as f64 * 0.02,
                30.25 + (index / 10) as f64 * 0.02,
            )
        })
        .collect::<Vec<_>>();
    let body = page_body(records, Some(100), None);
    let server = ScriptedServer::start(vec![(200, vec![], body, Duration::ZERO); 3]);
    let client = LoopbackODataClient::new(
        &format!("{}/odata/v1/", server.origin),
        Duration::from_secs(60),
    )
    .unwrap();
    let mut elapsed = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let response = client.search_all(&source, &|| false).unwrap();
        elapsed.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(response.products.len(), 100);
        assert_eq!(response.stats.exact_matched_count, 100);
        assert_eq!(response.stats.fetched_record_count, 100);
        assert_eq!(response.stats.request_count, 1);
        assert!(response.stats.remote_exhausted && response.stats.working_set_satisfied);
    }
    assert_eq!(server.requests.lock().unwrap().len(), 3);
    println!("CDSE_SPATIAL_100_SCENES_ELAPSED_MS={elapsed:?}");
}
