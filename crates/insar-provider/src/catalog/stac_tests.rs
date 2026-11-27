use super::*;

fn route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac".to_owned(),
        endpoint: "https://catalog.example.test/stac/v1/".to_owned(),
        search_path: "search".to_owned(),
        page_cap: 100,
        accepted_stac_versions: vec!["1.0.0".to_owned(), "1.1.0".to_owned()],
    }
}

fn binding() -> StacCollectionBinding {
    StacCollectionBinding {
        logical_dataset: "optical.fixture.l2".to_owned(),
        provider_id: "fixture.stac".to_owned(),
        native_collection_id: "fixture-l2".to_owned(),
        collection_profile_id: "fixture.stac.optical.l2.v1".to_owned(),
        identity_rule_id: "fixture.stac.item-id.v1".to_owned(),
        filename_rule_id: "fixture.stac.asset-filename.v1".to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind: DataSourceKind::Sentinel2,
        product_kind: ProductKind::OpticalScene,
        production_enabled: false,
        product_id_property: Some("scene:id".to_owned()),
        display_name_property: Some("title".to_owned()),
        queryables: vec![
            StacQueryableBinding {
                sceneport_filter_key: "cloud_cover_lte".to_owned(),
                stac_property: "eo:cloud_cover".to_owned(),
                operator: StacQueryOperator::Lte,
                value_kind: StacQueryValueKind::Number,
            },
            StacQueryableBinding {
                sceneport_filter_key: "platforms".to_owned(),
                stac_property: "platform".to_owned(),
                operator: StacQueryOperator::In,
                value_kind: StacQueryValueKind::StringList,
            },
            StacQueryableBinding {
                sceneport_filter_key: "quality".to_owned(),
                stac_property: "fixture:quality".to_owned(),
                operator: StacQueryOperator::Eq,
                value_kind: StacQueryValueKind::String,
            },
        ],
        asset_role_bindings: vec![
            StacAssetRoleBinding {
                stac_asset_key: "data".to_owned(),
                role: AssetRole::Primary,
                required: true,
                allowed_hosts: vec!["data.example.test".to_owned()],
            },
            StacAssetRoleBinding {
                stac_asset_key: "thumbnail".to_owned(),
                role: AssetRole::Thumbnail,
                required: false,
                allowed_hosts: vec!["preview.example.test".to_owned()],
            },
        ],
    }
}

fn polygon() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[120.0, 20.0], [121.0, 20.0], [121.0, 21.0], [120.0, 20.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn multipolygon() -> Map<String, Value> {
    json!({
        "type": "MultiPolygon",
        "coordinates": [[[[120.0, 20.0], [121.0, 20.0], [121.0, 21.0], [120.0, 20.0]]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn query(limit: Option<u64>) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: "optical.fixture.l2".to_owned(),
        provider_id: "fixture.stac".to_owned(),
        native_collection_id: "fixture-l2".to_owned(),
        aoi_geojson: Some(polygon()),
        cursor: None,
        result_limit: limit,
        filters: Map::from_iter([
            (
                "datetime".to_owned(),
                json!({"start":"2026-01-01T00:00:00Z","end":"2026-01-31T23:59:59Z"}),
            ),
            ("cloud_cover_lte".to_owned(), json!(20.5)),
            ("platforms".to_owned(), json!(["fixture-1", "fixture-2"])),
            ("quality".to_owned(), json!("validated")),
        ]),
    }
}

fn item(id: &str, stac_version: &str) -> Value {
    json!({
        "stac_version": stac_version,
        "stac_extensions": ["https://stac-extensions.github.io/projection/v2.0.0/schema.json"],
        "type": "Feature",
        "id": id,
        "collection": "fixture-l2",
        "geometry": polygon(),
        "properties": {
            "datetime": "2026-01-15T12:34:56Z",
            "scene:id": format!("native-{id}"),
            "title": format!("Fixture {id}"),
            "proj:epsg": 32651,
            "fixture:quality": "validated"
        },
        "assets": {
            "data": {
                "href": format!("https://data.example.test/archive/{id}.tif"),
                "type": "image/tiff; application=geotiff",
                "roles": ["data"],
                "file:size": 4096
            },
            "thumbnail": {
                "href": format!("https://preview.example.test/{id}.jpg"),
                "type": "image/jpeg",
                "roles": ["thumbnail"]
            },
            "qa": {
                "href": format!("https://ignored.example.test/{id}.json"),
                "type": "application/json"
            }
        },
        "links": [
            {"rel":"self","href":format!("https://catalog.example.test/stac/v1/collections/fixture-l2/items/{id}")},
            {"rel":"collection","href":"https://catalog.example.test/stac/v1/collections/fixture-l2"}
        ]
    })
}

fn page(features: Vec<Value>, next: Option<Value>) -> Value {
    let count = features.len();
    let mut links =
        vec![json!({"rel":"self","href":"https://catalog.example.test/stac/v1/search"})];
    if let Some(next) = next {
        links.push(next);
    }
    json!({
        "type":"FeatureCollection",
        "features":features,
        "numberReturned":count,
        "numberMatched":10,
        "links":links
    })
}

#[test]
fn reviewed_binding_projects_existing_collection_descriptor_without_production_admission() {
    let route = route();
    let binding = binding();
    binding.validate(&route).unwrap();
    let descriptor = binding.collection_descriptor();
    assert_eq!(descriptor.logical_dataset, "optical.fixture.l2");
    assert_eq!(descriptor.provider_id, "fixture.stac");
    assert_eq!(descriptor.native_collection_id, "fixture-l2");
    assert!(!descriptor.production_enabled);

    let mut sensitive_identity = binding.clone();
    sensitive_identity.product_id_property = Some("access_token".to_owned());
    assert_eq!(
        sensitive_identity.validate(&route).unwrap_err().code,
        "STAC_BINDING_INVALID"
    );

    let mut sensitive_asset = binding;
    sensitive_asset.asset_role_bindings[0].stac_asset_key = "refresh_token".to_owned();
    assert_eq!(
        sensitive_asset.validate(&route).unwrap_err().code,
        "STAC_BINDING_INVALID"
    );
}

#[test]
fn base_search_preserves_polygon_datetime_and_only_reviewed_queryables() {
    let request = build_stac_search_request(&route(), &binding(), &query(Some(25))).unwrap();
    assert_eq!(request.method, StacHttpMethod::Post);
    assert_eq!(
        request.url.as_str(),
        "https://catalog.example.test/stac/v1/search"
    );
    let body = request.body.unwrap();
    assert_eq!(body["collections"], json!(["fixture-l2"]));
    assert_eq!(body["intersects"], Value::Object(polygon()));
    assert_eq!(
        body["datetime"],
        "2026-01-01T00:00:00Z/2026-01-31T23:59:59Z"
    );
    assert_eq!(body["limit"], 25);
    assert_eq!(body["query"]["eo:cloud_cover"]["lte"], 20.5);
    assert_eq!(
        body["query"]["platform"]["in"],
        json!(["fixture-1", "fixture-2"])
    );
    assert_eq!(body["query"]["fixture:quality"]["eq"], "validated");
}

#[test]
fn base_search_preserves_multipolygon_without_bbox_substitution() {
    let mut request_query = query(Some(3));
    request_query.aoi_geojson = Some(multipolygon());
    let request = build_stac_search_request(&route(), &binding(), &request_query).unwrap();
    assert_eq!(
        request.body.unwrap()["intersects"],
        Value::Object(multipolygon())
    );
}

#[test]
fn undeclared_filter_and_noncanonical_datetime_fail_closed() {
    let mut invalid = query(Some(3));
    invalid
        .filters
        .insert("provider_magic".to_owned(), json!(true));
    assert_eq!(
        build_stac_search_request(&route(), &binding(), &invalid)
            .unwrap_err()
            .code,
        "STAC_QUERY_INVALID"
    );
    let mut invalid = query(Some(3));
    invalid.filters.insert(
        "datetime".to_owned(),
        json!({"start":"2026-01-01T08:00:00+08:00","end":"2026-01-31T23:59:59Z"}),
    );
    assert_eq!(
        build_stac_search_request(&route(), &binding(), &invalid)
            .unwrap_err()
            .code,
        "STAC_QUERY_INVALID"
    );
}

#[test]
fn item_accepts_stac_1_0_and_1_1_and_preserves_extension_metadata() {
    for version in ["1.0.0", "1.1.0"] {
        let product = map_stac_item(&route(), &binding(), &item("item-a", version)).unwrap();
        assert_eq!(product.product_id, "native-item-a");
        assert_eq!(product.display_name, "Fixture item-a");
        assert_eq!(
            product.acquisition_datetime.as_deref(),
            Some("2026-01-15T12:34:56Z")
        );
        assert_eq!(product.footprint_geojson, Some(polygon()));
        assert_eq!(product.properties["stac_properties"]["proj:epsg"], 32651);
        assert_eq!(product.payload["stac"]["version"], version);
        assert_eq!(
            product.payload["stac"]["unmapped_asset_keys"],
            json!(["qa"])
        );
        assert_eq!(product.assets.len(), 2);
        assert!(product
            .assets
            .iter()
            .all(|asset| asset.access_descriptor.is_none()));
    }
}

#[test]
fn collection_mismatch_and_missing_reviewed_identity_fail_closed() {
    let mut wrong_collection = item("item-a", "1.1.0");
    wrong_collection["collection"] = json!("other");
    assert_eq!(
        map_stac_item(&route(), &binding(), &wrong_collection)
            .unwrap_err()
            .code,
        "STAC_IDENTITY_MISMATCH"
    );
    let mut missing_identity = item("item-a", "1.1.0");
    missing_identity["properties"]
        .as_object_mut()
        .unwrap()
        .remove("scene:id");
    assert_eq!(
        map_stac_item(&route(), &binding(), &missing_identity)
            .unwrap_err()
            .code,
        "STAC_IDENTITY_MISMATCH"
    );
}

#[test]
fn item_preserves_multipolygon_footprint() {
    let mut feature = item("item-a", "1.1.0");
    feature["geometry"] = Value::Object(multipolygon());
    let product = map_stac_item(&route(), &binding(), &feature).unwrap();
    assert_eq!(product.footprint_geojson, Some(multipolygon()));
}

#[test]
fn reviewed_assets_get_stable_identity_but_not_transfer_authority() {
    let product = map_stac_item(&route(), &binding(), &item("item-a", "1.1.0")).unwrap();
    let data = product
        .assets
        .iter()
        .find(|asset| asset.asset_key.as_deref() == Some("data"))
        .unwrap();
    assert_eq!(data.role, AssetRole::Primary);
    assert_eq!(data.native_roles, vec!["data"]);
    assert!(data
        .stable_asset_key
        .as_deref()
        .unwrap()
        .contains("::ASSET::data"));
    assert_eq!(data.file_name.as_deref(), Some("item-a.tif"));
    assert_eq!(data.size_bytes, Some(4096));
    assert!(data.access_descriptor.is_none());
}

#[test]
fn required_asset_and_asset_host_are_fail_closed() {
    let mut missing = item("item-a", "1.1.0");
    missing["assets"].as_object_mut().unwrap().remove("data");
    assert_eq!(
        map_stac_item(&route(), &binding(), &missing)
            .unwrap_err()
            .code,
        "STAC_ASSET_INVALID"
    );
    let mut hostile = item("item-a", "1.1.0");
    hostile["assets"]["data"]["href"] = json!("https://evil.example.test/item-a.tif");
    assert_eq!(
        map_stac_item(&route(), &binding(), &hostile)
            .unwrap_err()
            .code,
        "STAC_ASSET_INVALID"
    );
    let mut signed = item("item-a", "1.1.0");
    signed["assets"]["data"]["href"] = json!("https://data.example.test/item-a.tif?sig=temporary");
    assert_eq!(
        map_stac_item(&route(), &binding(), &signed)
            .unwrap_err()
            .code,
        "STAC_ASSET_INVALID"
    );
    let mut sensitive_key = item("item-a", "1.1.0");
    sensitive_key["assets"].as_object_mut().unwrap().insert(
        "access_token".to_owned(),
        json!({"href":"https://ignored.example.test/opaque"}),
    );
    assert_eq!(
        map_stac_item(&route(), &binding(), &sensitive_key)
            .unwrap_err()
            .code,
        "STAC_ASSET_INVALID"
    );
}

#[test]
fn sensitive_extension_property_is_not_persisted() {
    let mut feature = item("item-a", "1.1.0");
    feature["properties"]["access_token"] = json!("fixture-secret");
    let error = map_stac_item(&route(), &binding(), &feature).unwrap_err();
    assert_eq!(error.code, "STAC_RESPONSE_INVALID");
    assert!(!format!("{error:?}").contains("fixture-secret"));

    let mut extension_query = item("item-a", "1.1.0");
    extension_query["stac_extensions"] =
        json!(["https://example.test/schema.json?token=temporary"]);
    assert_eq!(
        map_stac_item(&route(), &binding(), &extension_query)
            .unwrap_err()
            .code,
        "STAC_RESPONSE_INVALID"
    );
}

#[test]
fn get_next_link_becomes_query_bound_opaque_cursor() {
    let request_query = query(Some(2));
    let fingerprint = query_fingerprint(&route(), &binding(), &request_query).unwrap();
    let link = json!({
        "rel":"next",
        "href":"https://catalog.example.test/stac/v1/search?token=opaque-page-token"
    });
    let cursor = wrap_next_cursor(&route(), &fingerprint, link.as_object().unwrap(), None).unwrap();
    assert!(!cursor.contains("opaque-page-token"));
    let mut continued = request_query.clone();
    continued.cursor = Some(cursor);
    let request = build_stac_search_request(&route(), &binding(), &continued).unwrap();
    assert_eq!(request.method, StacHttpMethod::Get);
    assert_eq!(request.url.query(), Some("token=opaque-page-token"));
    assert!(!format!("{request:?}").contains("opaque-page-token"));

    continued.aoi_geojson = Some(multipolygon());
    assert_eq!(
        build_stac_search_request(&route(), &binding(), &continued)
            .unwrap_err()
            .code,
        "STAC_CURSOR_INVALID"
    );
}

#[test]
fn post_next_link_preserves_body_but_rejects_headers_and_secrets() {
    let request_query = query(Some(2));
    let fingerprint = query_fingerprint(&route(), &binding(), &request_query).unwrap();
    let link = json!({
        "rel":"next",
        "href":"/stac/v1/search",
        "method":"POST",
        "body":{"token":"page-2","collections":["fixture-l2"]}
    });
    let cursor = wrap_next_cursor(&route(), &fingerprint, link.as_object().unwrap(), None).unwrap();
    assert!(!cursor.contains("page-2"));
    let mut continued = request_query.clone();
    continued.cursor = Some(cursor);
    let request = build_stac_search_request(&route(), &binding(), &continued).unwrap();
    assert_eq!(request.method, StacHttpMethod::Post);
    assert_eq!(request.body.unwrap()["token"], "page-2");

    let request = build_stac_search_request(&route(), &binding(), &continued).unwrap();
    assert!(!format!("{request:?}").contains("page-2"));

    let headers = json!({
        "rel":"next","href":"/stac/v1/search","method":"POST",
        "headers":{},"body":{"token":"page-2"}
    });
    assert_eq!(
        wrap_next_cursor(&route(), &fingerprint, headers.as_object().unwrap(), None)
            .unwrap_err()
            .code,
        "STAC_CURSOR_INVALID"
    );
    let secret = json!({
        "rel":"next","href":"/stac/v1/search","method":"POST",
        "body":{"access_token":"fixture-secret"}
    });
    let error =
        wrap_next_cursor(&route(), &fingerprint, secret.as_object().unwrap(), None).unwrap_err();
    assert_eq!(error.code, "STAC_CURSOR_INVALID");
    assert!(!format!("{error:?}").contains("fixture-secret"));
}

#[test]
fn hostile_next_link_scope_and_method_fail_closed() {
    let request_query = query(Some(2));
    let fingerprint = query_fingerprint(&route(), &binding(), &request_query).unwrap();
    for link in [
        json!({"rel":"next","href":"https://evil.example.test/stac/v1/search?token=2"}),
        json!({"rel":"next","href":"https://user@catalog.example.test/stac/v1/search"}),
        json!({"rel":"next","href":"https://catalog.example.test/outside/search"}),
        json!({"rel":"next","href":"/stac/v1/search#fragment"}),
        json!({"rel":"next","href":"/stac/v1/search","method":"DELETE"}),
        // merge:true needs the original request body and a POST link.
        json!({"rel":"next","href":"/stac/v1/search","method":"POST","merge":true,"body":{}}),
        json!({"rel":"next","href":"/stac/v1/search?token=2","merge":true}),
    ] {
        assert_eq!(
            wrap_next_cursor(&route(), &fingerprint, link.as_object().unwrap(), None)
                .unwrap_err()
                .code,
            "STAC_CURSOR_INVALID"
        );
    }
}

#[test]
fn duplicate_next_links_and_duplicate_product_ids_fail_closed() {
    let duplicate_next = json!({
        "type":"FeatureCollection",
        "features":[item("a","1.1.0")],
        "numberReturned":1,
        "links":[
            {"rel":"next","href":"/stac/v1/search?token=2"},
            {"rel":"next","href":"/stac/v1/search?token=3"}
        ]
    });
    assert_eq!(
        map_stac_complete_search_response(&route(), &binding(), &query(Some(1)), &duplicate_next)
            .unwrap_err()
            .code,
        "STAC_CURSOR_INVALID"
    );
    let duplicate_items = page(vec![item("same", "1.1.0"), item("same", "1.1.0")], None);
    assert_eq!(
        map_stac_complete_search_response(&route(), &binding(), &query(Some(2)), &duplicate_items)
            .unwrap_err()
            .code,
        "STAC_IDENTITY_MISMATCH"
    );
}

#[test]
fn finite_satisfied_working_set_suppresses_cursor_but_preserves_remote_truth() {
    let response = map_stac_complete_search_response(
        &route(),
        &binding(),
        &query(Some(1)),
        &page(
            vec![item("a", "1.1.0")],
            Some(json!({"rel":"next","href":"/stac/v1/search?token=2"})),
        ),
    )
    .unwrap();
    assert_eq!(response.products.len(), 1);
    assert!(response.next_cursor.is_none());
    assert!(!response.stats.truncated);
    assert!(!response.stats.remote_exhausted);
    assert!(response.stats.working_set_satisfied);
    response.validate().unwrap();
}

#[test]
fn underfilled_or_unlimited_result_with_next_link_is_not_freezable() {
    let remote_next = Some(json!({"rel":"next","href":"/stac/v1/search?token=2"}));
    assert_eq!(
        map_stac_complete_search_response(
            &route(),
            &binding(),
            &query(Some(2)),
            &page(vec![item("a", "1.1.0")], remote_next.clone()),
        )
        .unwrap_err()
        .code,
        "STAC_RESPONSE_INVALID"
    );
    assert_eq!(
        map_stac_complete_search_response(
            &route(),
            &binding(),
            &query(None),
            &page(vec![item("a", "1.1.0")], remote_next),
        )
        .unwrap_err()
        .code,
        "STAC_RESPONSE_INVALID"
    );
}

#[test]
fn exhausted_page_is_complete_for_underfilled_and_unlimited_queries() {
    for limit in [Some(5), None] {
        let response = map_stac_complete_search_response(
            &route(),
            &binding(),
            &query(limit),
            &page(vec![item("a", "1.1.0")], None),
        )
        .unwrap();
        assert!(response.stats.remote_exhausted);
        assert!(response.stats.working_set_satisfied);
        assert!(response.next_cursor.is_none());
        response.validate().unwrap();
    }
}

#[test]
fn item_collection_counts_are_bounded_and_consistent() {
    let mut invalid = page(vec![item("a", "1.1.0")], None);
    invalid["numberReturned"] = json!(2);
    assert_eq!(
        map_stac_complete_search_response(&route(), &binding(), &query(Some(1)), &invalid)
            .unwrap_err()
            .code,
        "STAC_RESPONSE_INVALID"
    );
    let mut invalid = page(vec![item("a", "1.1.0")], None);
    invalid["numberMatched"] = json!(0);
    assert_eq!(
        map_stac_complete_search_response(&route(), &binding(), &query(Some(1)), &invalid)
            .unwrap_err()
            .code,
        "STAC_RESPONSE_INVALID"
    );
}
