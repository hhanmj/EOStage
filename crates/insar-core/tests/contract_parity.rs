use insar_core::{
    ApplicationSearchProgress, ApplicationSearchProgressStage, AsfSearchStats,
    AssetAccessDescriptor, AssetExecutionAuthority, AssetExecutionPolicy, BBox,
    CatalogCollectionDescriptor, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DataSourceKind, FrozenAssetAccess, FrozenSearchBatch, FrozenSearchStats,
    FrozenTaskSnapshot, FrozenTaskSnapshotV3, FrozenTransferOrigin, OrbitFileMetadata,
    OrbitParseRequest, OrbitPlatform, OrbitProductType, PreviewAssetRole, PreviewCachePolicy,
    PreviewCredentialRequirement, PreviewDescriptor, PreviewKind, ProviderMetadata, ProviderPlan,
    ProviderProduct, ProviderQuery, ResolvedPreviewAsset, SarBurstSelection, SarProduct,
    SarSearchQuery, SidecarRequest, SidecarResponse, SnapshotItem, StableAssetReference, TaskEvent,
    V3_CONTRACT_VERSION, V3_PROTOCOL_MAJOR, V3_PROTOCOL_MINOR, V3_PROTOCOL_MIN_MINOR,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/v3_contracts")
        .join(name)
}

#[test]
fn unified_search_request_requires_a_named_aoi_in_generated_wire_contract() {
    let payload = serde_json::json!({
        "resource_id": "science.hls.l30.v2", "aoi": {"type":"Polygon","coordinates":[]},
        "aoi_name": "测试区", "time_range": null, "filters": {}, "request_id": "fixture-1"
    });
    let request: insar_core::UnifiedSearchRequest =
        serde_json::from_value(payload.clone()).unwrap();
    assert_eq!(request.aoi_name, "测试区");
    assert!(request.validate().is_ok());
    for name in ["", "  \t "] {
        let mut invalid = payload.clone();
        invalid["aoi_name"] = serde_json::json!(name);
        assert!(
            serde_json::from_value::<insar_core::UnifiedSearchRequest>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut missing = payload;
    missing.as_object_mut().unwrap().remove("aoi_name");
    assert!(serde_json::from_value::<insar_core::UnifiedSearchRequest>(missing).is_err());
}

#[test]
fn landsat_source_kind_has_exact_wire_parity() {
    assert_eq!(
        serde_json::to_value(DataSourceKind::Landsat).unwrap(),
        Value::String("landsat".to_owned())
    );
    assert_eq!(
        serde_json::from_value::<DataSourceKind>(Value::String("landsat".to_owned())).unwrap(),
        DataSourceKind::Landsat
    );
}

fn assert_round_trip<T>(name: &str)
where
    T: DeserializeOwned + Serialize,
{
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(fixture_path(name)).expect("fixture readable"))
            .expect("fixture JSON valid");
    let model: T = serde_json::from_value(expected.clone()).expect("fixture deserializes");
    let actual = serde_json::to_value(model).expect("model serializes");
    assert_eq!(actual, expected);
}

#[test]
fn generated_contracts_round_trip_golden_json() {
    assert_round_trip::<ProviderMetadata>("provider_metadata.json");
    assert_round_trip::<ProviderQuery>("provider_query.json");
    assert_round_trip::<ProviderProduct>("provider_product.json");
    assert_round_trip::<ProviderPlan>("provider_plan.json");
    assert_round_trip::<ContractError>("contract_error.json");
    assert_round_trip::<TaskEvent>("task_event.json");
    assert_round_trip::<SidecarRequest>("sidecar_request.json");
    assert_round_trip::<SidecarResponse>("sidecar_response.json");
    assert_round_trip::<StableAssetReference>("stable_asset_reference.json");
    assert_round_trip::<AssetAccessDescriptor>("asset_access_descriptor.json");
    assert_round_trip::<FrozenAssetAccess>("frozen_asset_access.json");
}

#[test]
fn frozen_search_stats_are_optional_but_closed_when_present() {
    let legacy: FrozenSearchBatch = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "batch_id": "legacy-search-batch",
        "provider_id": "asf.catalog",
        "created_at": "2026-08-17T00:00:00Z",
        "aoi_name": "Legacy fixture AOI",
        "aoi_geojson": null,
        "query": {},
        "candidates": []
    }))
    .expect("legacy frozen batches remain readable without search statistics");
    assert!(legacy.search_stats.is_none());

    let stats = FrozenSearchStats {
        matched_count: Some(51_594),
        fetched_record_count: Some(20),
        mapped_record_count: Some(20),
        provider_returned_count: 20,
        frozen_candidate_count: 20,
        truncated: true,
        source: "ASF_BATCH".to_owned(),
        source_attempts: vec![],
    };
    stats.validate().expect("bounded search facts validate");

    let oversized_source = "x".repeat(257);
    for invalid_source in [
        "https://example.invalid/search",
        "Bearer credential",
        oversized_source.as_str(),
    ] {
        let mut invalid = stats.clone();
        invalid.source = invalid_source.to_owned();
        assert!(invalid.validate().is_err());
    }

    let invalid_stats_batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nested-search-stats".to_owned(),
        provider_id: "asf.catalog".to_owned(),
        created_at: "2026-08-17T00:00:00Z".to_owned(),
        aoi_name: "Nested validation AOI".to_owned(),
        aoi_geojson: None,
        query: Default::default(),
        search_stats: Some(FrozenSearchStats {
            source: "https://example.invalid/search".to_owned(),
            ..stats.clone()
        }),
        candidates: vec![],
    };
    assert_eq!(
        invalid_stats_batch.validate().unwrap_err(),
        "search_stats.source must be one of the declared values"
    );

    let traced: FrozenSearchStats = serde_json::from_value(serde_json::json!({
        "matched_count": 1,
        "fetched_record_count": 1,
        "mapped_record_count": 1,
        "provider_returned_count": 1,
        "frozen_candidate_count": 1,
        "truncated": false,
        "source": "TYPED_CATALOG",
        "source_attempts": [
            {
                "provider_id": "microsoft.planetary-computer",
                "native_collection_id": "landsat-c2-l2",
                "outcome": "fallback",
                "fallback_reason": "transient_network"
            },
            {
                "provider_id": "element84.earth-search",
                "native_collection_id": "landsat-c2-l2",
                "outcome": "succeeded",
                "fallback_reason": null
            }
        ]
    }))
    .unwrap();
    traced.validate().expect("bounded source trace validates");
    let traced_batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "source-trace".to_owned(),
        provider_id: "element84.earth-search".to_owned(),
        created_at: "2026-08-17T00:00:00Z".to_owned(),
        aoi_name: "Source trace AOI".to_owned(),
        aoi_geojson: None,
        query: Default::default(),
        search_stats: Some(traced.clone()),
        candidates: vec![],
    };
    traced_batch
        .validate()
        .expect("final trace provider matches batch");
    let mut wrong_final = traced_batch.clone();
    wrong_final.provider_id = "microsoft.planetary-computer".to_owned();
    assert_eq!(
        wrong_final.validate().unwrap_err(),
        "final source attempt provider must match the batch provider"
    );
    let mut bad_sequence = traced;
    bad_sequence.source_attempts[0].fallback_reason = None;
    assert_eq!(
        bad_sequence.validate().unwrap_err(),
        "source attempts before the final success must be bounded fallbacks"
    );

    let invalid_candidate: SnapshotItem = serde_json::from_value(serde_json::json!({
        "item_id": "",
        "provider_id": "asf.catalog",
        "display_name": "invalid candidate",
        "acquisition_datetime": null,
        "footprint_geojson": null,
        "metadata": {},
        "payload": {}
    }))
    .expect("structural invalid candidate deserializes before validation");
    let invalid_candidate_batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "nested-candidate".to_owned(),
        provider_id: "asf.catalog".to_owned(),
        created_at: "2026-08-17T00:00:00Z".to_owned(),
        aoi_name: "Nested validation AOI".to_owned(),
        aoi_geojson: None,
        query: Default::default(),
        search_stats: None,
        candidates: vec![invalid_candidate],
    };
    assert_eq!(
        invalid_candidate_batch.validate().unwrap_err(),
        "candidates[0].item_id is too short"
    );

    let mut unknown = serde_json::to_value(stats).expect("search facts serialize");
    unknown["remote_url"] = serde_json::json!("https://example.invalid/forbidden");
    assert!(serde_json::from_value::<FrozenSearchStats>(unknown).is_err());
}

#[test]
fn catalog_search_query_uses_nullable_result_limit_not_transport_page_size() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(fixture_path("catalog_contract_cases.json")).expect("fixture readable"),
    )
    .expect("fixture JSON valid");
    let mut base = fixture["query"].clone();
    base.as_object_mut()
        .expect("query fixture is an object")
        .remove("result_limit");

    for result_limit in [
        serde_json::json!(1),
        serde_json::json!(17),
        serde_json::json!(250),
        serde_json::json!(251),
        serde_json::json!(617),
        serde_json::json!(3000),
        serde_json::json!(10_000),
        Value::Null,
    ] {
        let mut value = base.clone();
        value["result_limit"] = result_limit.clone();
        let query: CatalogSearchQuery =
            serde_json::from_value(value).expect("reviewed result limit deserializes");
        query.validate().expect("reviewed result limit validates");
        let serialized = serde_json::to_value(query).expect("query serializes");
        assert_eq!(serialized["result_limit"], result_limit);
        assert!(serialized.get("page_size").is_none());
    }

    assert!(serde_json::from_value::<CatalogSearchQuery>(base.clone()).is_err());
    for result_limit in [
        serde_json::json!(0),
        serde_json::json!(1.5),
        serde_json::json!("17"),
        serde_json::json!("not-a-number"),
        serde_json::json!(10_001),
    ] {
        let mut value = base.clone();
        value["result_limit"] = result_limit;
        assert!(serde_json::from_value::<CatalogSearchQuery>(value)
            .and_then(|query| query.validate().map_err(serde::de::Error::custom))
            .is_err());
    }
}

#[test]
fn application_search_progress_preserves_unknown_zero_and_lifecycle_invariants() {
    let validating = ApplicationSearchProgress {
        request_id: "search-1".to_owned(),
        sequence: 1,
        stage: ApplicationSearchProgressStage::Validating,
        provider_request_count: None,
        raw_fetched_count: None,
        mapped_count: None,
        exact_matched_count: None,
        admitted_count: None,
        requested_limit: Some(25),
        remote_total: None,
        batch_id: None,
        loaded_count: None,
        elapsed_ms: 0,
        message: "Validating search request".to_owned(),
        terminal: false,
    };
    validating.validate().expect("initial progress validates");
    let serialized = serde_json::to_value(&validating).expect("progress serializes");
    assert!(serialized["provider_request_count"].is_null());

    let mut observed_zero = validating.clone();
    observed_zero.sequence = 2;
    observed_zero.stage = ApplicationSearchProgressStage::Fetching;
    observed_zero.provider_request_count = Some(1);
    observed_zero.raw_fetched_count = Some(0);
    observed_zero.mapped_count = Some(0);
    observed_zero.exact_matched_count = Some(0);
    observed_zero.admitted_count = Some(0);
    observed_zero.message = "Provider returned no records".to_owned();
    observed_zero
        .validate()
        .expect("observed zero remains distinct from unknown");
    let round_trip: ApplicationSearchProgress = serde_json::from_value(
        serde_json::to_value(observed_zero).expect("observed progress serializes"),
    )
    .expect("observed progress deserializes");
    assert_eq!(round_trip.raw_fetched_count, Some(0));

    let mut terminal_mismatch = validating.clone();
    terminal_mismatch.stage = ApplicationSearchProgressStage::Completed;
    assert!(terminal_mismatch.validate().is_err());

    let mut invalid_order = validating;
    invalid_order.raw_fetched_count = Some(1);
    invalid_order.mapped_count = Some(2);
    assert!(invalid_order.validate().is_err());
}

#[test]
fn generic_catalogue_contracts_preserve_closed_roles_and_durable_oauth_access() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(fixture_path("catalog_contract_cases.json")).expect("fixture readable"),
    )
    .expect("fixture JSON valid");

    for aoi_geojson in fixture["valid_polygonal_aoi_geojson"]
        .as_array()
        .expect("valid polygonal AOI cases are an array")
    {
        let mut valid_query = fixture["query"].clone();
        valid_query["aoi_geojson"] = aoi_geojson.clone();
        let query = serde_json::from_value::<CatalogSearchQuery>(valid_query)
            .expect("reviewed polygonal AOI topology deserializes");
        query
            .validate()
            .expect("reviewed polygonal AOI topology validates");
        assert_eq!(
            serde_json::to_value(&query).expect("query serializes")["aoi_geojson"],
            aoi_geojson.clone()
        );
    }

    let collection: CatalogCollectionDescriptor =
        serde_json::from_value(fixture["collection"].clone()).expect("collection deserializes");
    let query: CatalogSearchQuery =
        serde_json::from_value(fixture["query"].clone()).expect("query deserializes");
    let response: CatalogSearchResponse =
        serde_json::from_value(fixture["response"].clone()).expect("response deserializes");
    let deduplicated: CatalogSearchResponse =
        serde_json::from_value(fixture["deduplicated_response"].clone())
            .expect("deduplicated response deserializes");
    let descriptor: AssetAccessDescriptor =
        serde_json::from_value(fixture["oauth_descriptor"].clone())
            .expect("OAuth descriptor deserializes");
    let old_asset: insar_core::ProviderAsset =
        serde_json::from_value(fixture["old_asset"].clone()).expect("old asset deserializes");
    let native_role_assets: Vec<insar_core::ProviderAsset> =
        serde_json::from_value(fixture["native_role_assets"].clone())
            .expect("native role fixtures deserialize");

    assert_eq!(collection.logical_dataset, "optical.sentinel-2.l2a");
    assert_eq!(query.native_collection_id, "SENTINEL-2");
    assert_eq!(
        query
            .aoi_geojson
            .as_ref()
            .expect("fixture query includes AOI")["type"],
        "Polygon"
    );
    assert_eq!(response.stats.returned_count, 2);
    assert_eq!(response.stats.exact_duplicate_record_count, 0);
    assert_eq!(response.stats.unique_provider_product_count, 2);
    deduplicated
        .validate()
        .expect("exact duplicate diagnostics remain auditable");
    assert_eq!(deduplicated.stats.fetched_record_count, 3);
    assert_eq!(deduplicated.stats.exact_duplicate_record_count, 1);
    assert_eq!(deduplicated.stats.unique_provider_product_count, 2);
    assert_eq!(response.next_cursor.as_deref(), Some("fixture-page-2"));
    assert_eq!(
        response.products[0].footprint_geojson.as_ref().unwrap()["type"],
        "Polygon"
    );
    assert_eq!(old_asset.native_roles, Vec::<String>::new());
    assert_eq!(
        native_role_assets[0].native_roles,
        ["thumbnail", "overview"]
    );
    assert_eq!(
        native_role_assets[1].native_roles,
        ["unreviewed-native-role"]
    );
    assert_ne!(
        native_role_assets[0].asset_key,
        native_role_assets[2].asset_key
    );
    descriptor
        .validate()
        .expect("durable OAuth descriptor validates");
    for stable_href in fixture["valid_oauth_hrefs"]
        .as_array()
        .expect("valid OAuth href cases are an array")
    {
        let mut valid = fixture["oauth_descriptor"].clone();
        valid["stable_href"] = stable_href.clone();
        serde_json::from_value::<AssetAccessDescriptor>(valid)
            .expect("valid OAuth shape deserializes")
            .validate()
            .expect("valid OAuth locator validates");
    }
    let mut feature_footprint = serde_json::to_value(&response).expect("response serializes");
    feature_footprint["products"][0]["footprint_geojson"] =
        fixture["valid_feature_footprint_geojson"].clone();
    let feature_footprint: CatalogSearchResponse =
        serde_json::from_value(feature_footprint).expect("feature footprint deserializes");
    feature_footprint
        .validate()
        .expect("ordinary footprint Feature properties validate");
    assert_eq!(
        feature_footprint.products[0]
            .footprint_geojson
            .as_ref()
            .expect("feature footprint remains available")["properties"]["ordinary_label"],
        "preserve-this-property"
    );

    let mut exhaustive = serde_json::to_value(&response).expect("response serializes");
    exhaustive["stats"]["requested_limit"] = Value::Null;
    let exhaustive: CatalogSearchResponse =
        serde_json::from_value(exhaustive).expect("ALL keeps requested_limit explicitly null");
    assert!(
        serde_json::to_value(exhaustive).expect("ALL response serializes")["stats"]
            ["requested_limit"]
            .is_null()
    );

    let invalid_stats = CatalogSearchStats {
        requested_limit: Some(1),
        matched_count: Some(1),
        request_count: 1,
        transport_part_count: 1,
        fetched_record_count: 1,
        mapped_record_count: 1,
        profile_filtered_record_count: 0,
        exact_duplicate_record_count: 0,
        unique_provider_product_count: 1,
        exact_matched_count: 1,
        returned_count: 2,
        remote_exhausted: true,
        working_set_satisfied: true,
        elapsed_ms: 1,
        truncated: false,
    };
    assert!(invalid_stats.validate().is_err());

    let mut missing_profile = descriptor.clone();
    missing_profile.credential_profile = None;
    assert!(missing_profile.validate().is_err());
    let public_with_credentials: AssetAccessDescriptor =
        serde_json::from_value(serde_json::json!({
            "resolver_id": "public_https",
            "scheme": "public_https",
            "stable_href": "https://example.invalid/archive.zip",
            "requires_credentials": true,
            "credential_profile": "not-allowed",
            "resolver_parameters": {}
        }))
        .expect("structural descriptor deserializes");
    assert!(public_with_credentials.validate().is_err());
}

#[test]
fn generic_catalogue_contracts_fail_closed_for_geometry_identity_and_oauth_locator() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(fixture_path("catalog_contract_cases.json")).expect("fixture readable"),
    )
    .expect("fixture JSON valid");

    let no_aoi_query = serde_json::from_value::<CatalogSearchQuery>(
        fixture["invalid_catalogue_cases"]["bbox_only_query"].clone(),
    )
    .expect("no-AOI catalogue query remains a valid contract");
    no_aoi_query
        .validate()
        .expect("no-AOI catalogue query validates without inventing geometry");

    for invalid_query in [
        fixture["invalid_catalogue_cases"]["point_query"].clone(),
        fixture["invalid_catalogue_cases"]["line_string_query"].clone(),
        fixture["invalid_catalogue_cases"]["empty_polygon_query"].clone(),
    ] {
        assert!(serde_json::from_value::<CatalogSearchQuery>(invalid_query)
            .and_then(|query| query.validate().map_err(serde::de::Error::custom))
            .is_err());
    }
    for invalid_aoi in [
        fixture["invalid_catalogue_cases"]["sensitive_aoi_geojson"].clone(),
        fixture["invalid_catalogue_cases"]["crs_aoi_geojson"].clone(),
    ] {
        let mut invalid_query = fixture["query"].clone();
        invalid_query["aoi_geojson"] = invalid_aoi;
        assert!(serde_json::from_value::<CatalogSearchQuery>(invalid_query)
            .and_then(|query| query.validate().map_err(serde::de::Error::custom))
            .is_err());
    }

    let response: CatalogSearchResponse =
        serde_json::from_value(fixture["response"].clone()).expect("valid fixture response");
    for invalid_footprint_geojson in [
        fixture["invalid_catalogue_cases"]["invalid_footprint_geojson"].clone(),
        fixture["invalid_catalogue_cases"]["sensitive_footprint_geojson"].clone(),
        fixture["invalid_catalogue_cases"]["crs_footprint_geojson"].clone(),
    ] {
        let mut invalid_footprint = serde_json::to_value(&response).expect("response serializes");
        invalid_footprint["products"][0]["footprint_geojson"] = invalid_footprint_geojson;
        let invalid_footprint: CatalogSearchResponse =
            serde_json::from_value(invalid_footprint).expect("invalid geometry retains Rust shape");
        assert!(invalid_footprint.validate().is_err());
    }
    let deduplicated: CatalogSearchResponse =
        serde_json::from_value(fixture["deduplicated_response"].clone())
            .expect("deduplicated response deserializes");
    for (field, value) in [
        ("exact_duplicate_record_count", Value::from(0)),
        ("unique_provider_product_count", Value::from(1)),
    ] {
        let mut invalid = serde_json::to_value(&deduplicated).expect("response serializes");
        invalid["stats"][field] = value;
        let invalid: CatalogSearchResponse =
            serde_json::from_value(invalid).expect("invalid statistics retain Rust shape");
        assert!(invalid.validate().is_err());
    }
    let mut invalid_mapping_failures =
        serde_json::to_value(&response).expect("response serializes");
    invalid_mapping_failures["item_errors"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "code": "FIXTURE_MAPPING_ERROR",
            "user_message": "synthetic per-record mapping failure",
            "debug_context": {}
        }));
    let invalid_mapping_failures: CatalogSearchResponse =
        serde_json::from_value(invalid_mapping_failures)
            .expect("invalid mapping count retains Rust shape");
    assert!(invalid_mapping_failures.validate().is_err());
    for (pointer, value) in [
        (
            "/products/1/provider_id",
            Value::String("wrong.provider".to_string()),
        ),
        (
            "/products/1/product_id",
            Value::String("fixture-uuid-1".to_string()),
        ),
        (
            "/products/0/assets/0/asset_key",
            Value::String("duplicate".to_string()),
        ),
    ] {
        let mut invalid = serde_json::to_value(&response).expect("response serializes");
        *invalid
            .pointer_mut(pointer)
            .expect("fixture pointer exists") = value;
        if pointer.ends_with("asset_key") {
            invalid["products"][0]["assets"].as_array_mut().unwrap().push(serde_json::json!({
                "asset_key": "duplicate", "stable_asset_key": "different-stable-key", "role": "browse",
                "native_roles": ["overview"], "url": null, "file_name": null,
                "media_type": null, "size_bytes": null
            }));
        }
        let invalid: CatalogSearchResponse =
            serde_json::from_value(invalid).expect("identity shape still deserializes");
        assert!(invalid.validate().is_err());
    }

    let mut duplicate_stable = serde_json::to_value(&response).expect("response serializes");
    duplicate_stable["products"][0]["assets"]
        .as_array_mut()
        .expect("assets are an array")
        .push(serde_json::json!({
            "asset_key": "different-asset", "stable_asset_key": "PROVIDER::fixture::COLLECTION::SENTINEL-2::ITEM::fixture-uuid-1::ASSET::archive-1",
            "role": "browse", "native_roles": ["overview"], "url": null,
            "file_name": null, "media_type": null, "size_bytes": null
        }));
    let duplicate_stable: CatalogSearchResponse =
        serde_json::from_value(duplicate_stable).expect("stable-key conflict deserializes");
    assert!(duplicate_stable.validate().is_err());

    for role in fixture["invalid_catalogue_cases"]["invalid_native_roles"]
        .as_array()
        .expect("native role cases are an array")
    {
        let mut invalid = serde_json::to_value(&response).expect("response serializes");
        invalid["products"][0]["assets"][0]["native_roles"] = serde_json::json!([role]);
        assert!(serde_json::from_value::<CatalogSearchResponse>(invalid)
            .and_then(|response| response.validate().map_err(serde::de::Error::custom))
            .is_err());
    }

    for stable_href in fixture["invalid_catalogue_cases"]["invalid_oauth_hrefs"]
        .as_array()
        .expect("OAuth cases are an array")
    {
        let mut invalid = fixture["oauth_descriptor"].clone();
        invalid["stable_href"] = stable_href.clone();
        let descriptor: AssetAccessDescriptor =
            serde_json::from_value(invalid).expect("OAuth shape remains deserializable");
        assert!(descriptor.validate().is_err());
    }
}

#[test]
fn frozen_asset_access_requires_checksum_algorithm_and_value_together() {
    let fixture =
        fs::read_to_string(fixture_path("frozen_asset_access.json")).expect("fixture readable");
    let valid: FrozenAssetAccess = serde_json::from_str(&fixture).expect("fixture deserializes");
    valid.validate().expect("typed checksum pair validates");

    for (algorithm, value) in [
        (Some("sha256".to_string()), None),
        (None, Some("fixture-checksum".to_string())),
    ] {
        let mut invalid = valid.clone();
        invalid.checksum_algorithm = algorithm;
        invalid.checksum_value = value;
        assert_eq!(
            invalid.validate().unwrap_err(),
            "checksum_algorithm presence must match checksum_value"
        );
    }
}

#[test]
fn asset_execution_policy_enforces_authority_shape() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(fixture_path("asset_execution_policy.json")).expect("fixture readable"),
    )
    .expect("fixture JSON valid");
    let legacy: AssetExecutionPolicy =
        serde_json::from_value(fixture["legacy"].clone()).expect("legacy policy deserializes");
    let resolver: AssetExecutionPolicy =
        serde_json::from_value(fixture["resolver"].clone()).expect("resolver policy deserializes");

    assert_eq!(legacy.authority, AssetExecutionAuthority::Legacy);
    legacy.validate().expect("legacy policy validates");
    assert_eq!(
        resolver.authority,
        AssetExecutionAuthority::ResolverAuthoritative
    );
    resolver.validate().expect("resolver policy validates");

    for invalid in [
        serde_json::json!({
            "authority": "legacy",
            "policy_id": "asf.sentinel-1.standard.resolver-cutover.v1",
            "collection_profile_id": null
        }),
        serde_json::json!({
            "authority": "resolver_authoritative",
            "policy_id": "legacy.v1",
            "collection_profile_id": "asf.sentinel-1.standard.v1"
        }),
        serde_json::json!({
            "authority": "resolver_authoritative",
            "policy_id": "asf.sentinel-1.standard.resolver-cutover.v1",
            "collection_profile_id": null
        }),
    ] {
        let policy: AssetExecutionPolicy =
            serde_json::from_value(invalid).expect("structural policy shape deserializes");
        assert!(policy.validate().is_err());
    }
}

#[test]
fn asset_access_descriptor_rejects_signed_or_secret_bearing_locators() {
    let mut descriptor: AssetAccessDescriptor = serde_json::from_value(serde_json::json!({
        "resolver_id": "public_https",
        "scheme": "public_https",
        "stable_href": "https://example.org/file.zip",
        "requires_credentials": false,
        "credential_profile": null,
        "resolver_parameters": {}
    }))
    .expect("durable descriptor deserializes");
    descriptor.validate().expect("durable descriptor validates");
    descriptor.stable_href =
        Some("https://example.org/file.zip?X-Amz-Signature=SECRET".to_string());
    assert!(descriptor.validate().is_err());
}

#[test]
fn generated_sar_ranges_and_burst_assets_round_trip() {
    let query: SarSearchQuery = serde_json::from_value(serde_json::json!({
        "dataset": "SENTINEL-1",
        "track_range": {"start": "10", "end": "20"},
        "frame_range": {"start": "001", "end": "010"}
    }))
    .expect("SAR range query deserializes");
    let query_value = serde_json::to_value(query).expect("SAR range query serializes");
    assert_eq!(query_value["track_range"]["start"], "10");
    assert_eq!(query_value["frame_range"]["end"], "010");

    let product: SarProduct = serde_json::from_value(serde_json::json!({
        "product_key": "ASF::SLC-BURST::file-id",
        "provider_product_key": "ASF::SLC-BURST::file-id",
        "provider_product_id": "file-id",
        "canonical_product_key": null,
        "identity_rule_id": "asf.sentinel1-burst.v1",
        "product_id": "file-id",
        "provider": "ASF",
        "dataset": "SLC-BURST",
        "burst": {
            "full_burst_id": "001_000001_IW1",
            "relative_burst_id": 1,
            "absolute_burst_id": 1001,
            "burst_index": 2
        },
        "assets": [{
            "asset_key": "burst-tiff",
            "stable_asset_key": "ASF::SLC-BURST::file-id::ASSET::burst-tiff",
            "role": "primary",
            "url": "https://example.invalid/burst.tiff",
            "file_name": "burst.tiff",
            "media_type": "image/tiff",
            "size_bytes": 42
        }]
    }))
    .expect("Burst product deserializes");
    let product_value = serde_json::to_value(product).expect("Burst product serializes");
    assert_eq!(product_value["burst"]["full_burst_id"], "001_000001_IW1");
    assert_eq!(product_value["burst"]["relative_burst_id"], 1);
    assert_eq!(product_value["burst"]["absolute_burst_id"], 1001);
    assert_eq!(product_value["assets"][0]["file_name"], "burst.tiff");
    assert_eq!(
        product_value["provider_product_key"],
        product_value["product_key"]
    );
    assert_eq!(product_value["assets"][0]["asset_key"], "burst-tiff");
}

#[test]
fn legacy_sar_payloads_default_new_optional_fields() {
    let query: SarSearchQuery = serde_json::from_value(serde_json::json!({
        "dataset": "SENTINEL-1",
        "relative_orbit": 10,
        "frame": 1
    }))
    .expect("legacy scalar-only query deserializes");
    assert_eq!(query.track_range, None);
    assert_eq!(query.frame_range, None);

    let burst: SarBurstSelection = serde_json::from_value(serde_json::json!({
        "burst_id": "legacy-burst-id"
    }))
    .expect("legacy Burst selection deserializes");
    assert_eq!(burst.full_burst_id, None);
    assert_eq!(burst.relative_burst_id, None);
    assert_eq!(burst.absolute_burst_id, None);

    let product: SarProduct = serde_json::from_value(serde_json::json!({
        "product_key": "ASF::SENTINEL-1::legacy-product",
        "product_id": "legacy-product",
        "provider": "ASF",
        "dataset": "SENTINEL-1"
    }))
    .expect("legacy SAR product deserializes");
    assert!(product.assets.is_empty());
    assert_eq!(product.provider_product_key, None);
    assert_eq!(product.provider_product_id, None);
    assert_eq!(product.canonical_product_key, None);
    assert_eq!(product.identity_rule_id, None);
}

#[test]
fn generated_sidecar_protocol_versions_and_response_invariants() {
    assert_eq!(V3_PROTOCOL_MAJOR, 1);
    assert_eq!(V3_CONTRACT_VERSION, "3.43.0");
    assert_eq!(V3_PROTOCOL_MINOR, 42);
    assert_eq!(V3_PROTOCOL_MIN_MINOR, 4);

    let usgs_scheme: insar_core::AssetAccessScheme =
        serde_json::from_str("\"usgs_m2m\"").expect("3.16 USGS M2M access scheme");
    assert_eq!(
        serde_json::to_value(usgs_scheme).unwrap(),
        serde_json::json!("usgs_m2m")
    );

    let preview: PreviewDescriptor = serde_json::from_value(serde_json::json!({
        "preview_kind": "quicklook",
        "source_asset_keys": ["ASF::SENTINEL-1::fixture::ASSET::browse"],
        "render_profile_id": "sar.provider-quicklook.v1",
        "media_type": "image/jpeg",
        "credential_requirement": "none",
        "cache_policy": "session",
        "provenance": "ASF provider browse asset"
    }))
    .expect("Preview descriptor deserializes");
    assert_eq!(preview.preview_kind, PreviewKind::Quicklook);

    let mut response: SidecarResponse =
        serde_json::from_str(&fs::read_to_string(fixture_path("sidecar_response.json")).unwrap())
            .unwrap();
    response.ok = false;
    assert_eq!(
        response.validate().unwrap_err(),
        "failed response requires an error"
    );

    response.ok = true;
    response.error = Some(ContractError {
        code: "V3_TEST".to_string(),
        user_message: "test".to_string(),
        debug_context: serde_json::Map::new(),
    });
    assert_eq!(
        response.validate().unwrap_err(),
        "successful response cannot contain an error"
    );
}

#[test]
fn resolved_preview_asset_accepts_only_renderable_source_roles() {
    let descriptor = PreviewDescriptor {
        preview_kind: PreviewKind::Quicklook,
        source_asset_keys: vec!["fixture::preview-discovery".to_owned()],
        render_profile_id: "fixture.preview.v1".to_owned(),
        media_type: "image/jpeg".to_owned(),
        credential_requirement: PreviewCredentialRequirement::None,
        cache_policy: PreviewCachePolicy::NoStore,
        provenance: "fixture rendered preview".to_owned(),
    };
    for source_asset_role in [
        PreviewAssetRole::Preview,
        PreviewAssetRole::Browse,
        PreviewAssetRole::Thumbnail,
    ] {
        ResolvedPreviewAsset {
            product_key: "fixture-product".to_owned(),
            descriptor: descriptor.clone(),
            source_asset_role,
            source_asset_key: "fixture::preview-discovery".to_owned(),
            media_type: "image/jpeg".to_owned(),
            data_url: "data:image/jpeg;base64,/9j/".to_owned(),
            byte_length: 3,
        }
        .validate()
        .expect("renderable Preview role validates");
    }
    for source_asset_role in [
        PreviewAssetRole::Primary,
        PreviewAssetRole::Metadata,
        PreviewAssetRole::Auxiliary,
    ] {
        assert_eq!(
            ResolvedPreviewAsset {
                product_key: "fixture-product".to_owned(),
                descriptor: descriptor.clone(),
                source_asset_role,
                source_asset_key: "fixture::preview-discovery".to_owned(),
                media_type: "image/jpeg".to_owned(),
                data_url: "data:image/jpeg;base64,/9j/".to_owned(),
                byte_length: 3,
            }
            .validate()
            .unwrap_err(),
            "source_asset_role must be renderable Preview evidence"
        );
    }
}

#[test]
fn dormant_v3_frozen_task_contract_is_closed_and_keeps_v2_unchanged() {
    let source_access = serde_json::json!({
        "reference": {
            "provider_id": "fixture-asf",
            "collection_id": "SENTINEL-1",
            "provider_product_id": "source-1",
            "provider_product_key": "fixture:source-1",
            "canonical_product_key": null,
            "asset_key": "slc",
            "stable_asset_key": "fixture:source-1:slc"
        },
        "descriptor": {
            "resolver_id": "fixture.asf.v1",
            "scheme": "public_https",
            "stable_href": null,
            "requires_credentials": false,
            "credential_profile": null,
            "resolver_parameters": {}
        },
        "role": "primary",
        "file_name": "source-1.zip",
        "expected_size_bytes": 1024,
        "checksum_algorithm": null,
        "checksum_value": null
    });
    let orbit_access = serde_json::json!({
        "reference": {
            "provider_id": "fixture-esa",
            "collection_id": "AUX_POEORB",
            "provider_product_id": "poeorb-1",
            "provider_product_key": "fixture:poeorb-1",
            "canonical_product_key": null,
            "asset_key": "orbit",
            "stable_asset_key": "fixture:poeorb-1:orbit"
        },
        "descriptor": {
            "resolver_id": "fixture.esa.poeorb.v1",
            "scheme": "public_https",
            "stable_href": null,
            "requires_credentials": false,
            "credential_profile": null,
            "resolver_parameters": {}
        },
        "role": "auxiliary",
        "file_name": "S1A_OPER_AUX_POEORB_FIXTURE.EOF",
        "expected_size_bytes": 2048,
        "checksum_algorithm": null,
        "checksum_value": null
    });
    let payload = serde_json::json!({
        "persistence_version": 3,
        "schema_version": 1,
        "task_id": "fixture-v3-task",
        "batch_id": "fixture-batch",
        "provider_id": "fixture-asf",
        "created_at": "2025-01-16T02:00:00Z",
        "aoi_name": "Fixture AOI",
        "aoi_geojson": {"type": "Polygon", "coordinates": []},
        "query": {"dataset": "SENTINEL-1"},
        "source_items": [{
            "item_id": "source-1",
            "provider_id": "fixture-asf",
            "display_name": "source-1.zip",
            "acquisition_datetime": "2025-01-15T01:02:03Z",
            "footprint_geojson": {"type": "Polygon", "coordinates": []},
            "metadata": {},
            "payload": {},
            "asset_access": source_access.clone(),
            "execution_policy": null
        }],
        "initial_selected_item_ids": ["source-1"],
        "output_root": "fixture-output-root",
        "layout": "sentinel1_slc",
        "runtime_item_identity": "transfer_item.v1",
        "transfer_items": [{
            "transfer_item_id": "primary-source-1",
            "source_item_ids": ["source-1"],
            "origin": "selected_product",
            "asset_access": source_access,
            "required": true
        }, {
            "transfer_item_id": "poeorb-shared",
            "source_item_ids": ["source-1"],
            "origin": "required_companion",
            "asset_access": orbit_access,
            "required": true
        }],
        "orbit_associations": [{
            "association_version": 1,
            "source_item_id": "source-1",
            "orbit_transfer_item_id": "poeorb-shared",
            "requested_orbit_type": "POEORB",
            "source_platform": "S1A",
            "source_acquisition_start": 1736902923,
            "source_acquisition_stop": 1736902983,
            "matched_orbit_metadata": {
                "file_name": "S1A_OPER_AUX_POEORB_FIXTURE.EOF",
                "platform": "S1A",
                "orbit_type": "POEORB",
                "creation_unix_seconds": 1736985600,
                "validity_start_unix_seconds": 1736726400,
                "validity_stop_unix_seconds": 1737072000
            },
            "selection_policy_id": "sentinel1-poeorb-strict-v1"
        }]
    });

    let snapshot: FrozenTaskSnapshotV3 =
        serde_json::from_value(payload.clone()).expect("V3 snapshot deserializes");
    snapshot.validate().expect("V3 structural shape validates");
    assert_eq!(serde_json::to_value(&snapshot).unwrap(), payload);
    assert_eq!(snapshot.persistence_version, 3);
    assert_eq!(snapshot.runtime_item_identity, "transfer_item.v1");
    assert_eq!(
        snapshot.transfer_items[0].origin,
        FrozenTransferOrigin::SelectedProduct
    );

    let mut invalid_constant = snapshot.clone();
    invalid_constant.persistence_version = 2;
    assert!(invalid_constant.validate().is_err());
    let mut invalid_orbit_type = snapshot.clone();
    invalid_orbit_type.orbit_associations[0].requested_orbit_type = "MOEORB".to_string();
    assert!(invalid_orbit_type.validate().is_err());
    let mut invalid_file_name = snapshot.clone();
    invalid_file_name.transfer_items[0]
        .asset_access
        .file_name
        .clear();
    assert!(invalid_file_name.validate().is_err());

    let mut empty_array = payload.clone();
    empty_array["source_items"] = serde_json::json!([]);
    let empty: FrozenTaskSnapshotV3 =
        serde_json::from_value(empty_array).expect("empty array retains Rust shape");
    assert!(empty.validate().is_err());

    let mut empty_output_root = payload.clone();
    empty_output_root["output_root"] = serde_json::json!("");
    let empty_path: FrozenTaskSnapshotV3 =
        serde_json::from_value(empty_output_root).expect("empty path retains Rust shape");
    assert!(empty_path.validate().is_err());

    let mut missing_required = payload.clone();
    missing_required
        .as_object_mut()
        .unwrap()
        .remove("transfer_items");
    assert!(serde_json::from_value::<FrozenTaskSnapshotV3>(missing_required).is_err());
    let mut missing_access = payload.clone();
    missing_access["source_items"][0]
        .as_object_mut()
        .unwrap()
        .remove("asset_access");
    assert!(serde_json::from_value::<FrozenTaskSnapshotV3>(missing_access).is_err());
    let mut null_file_name = payload.clone();
    null_file_name["transfer_items"][0]["asset_access"]["file_name"] = serde_json::json!(null);
    assert!(serde_json::from_value::<FrozenTaskSnapshotV3>(null_file_name).is_err());
    let mut outer_role = payload.clone();
    outer_role["transfer_items"][0]["role"] = serde_json::json!("primary");
    assert!(serde_json::from_value::<FrozenTaskSnapshotV3>(outer_role).is_err());
    let mut unknown = payload.clone();
    unknown["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<FrozenTaskSnapshotV3>(unknown).is_err());

    let legacy: FrozenTaskSnapshot = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "task_id": "legacy-task",
        "batch_id": "legacy-batch",
        "provider_id": "fixture-asf",
        "created_at": "2025-01-16T02:00:00Z",
        "aoi_name": "Fixture AOI",
        "aoi_geojson": {"type": "Polygon", "coordinates": []},
        "query": {},
        "candidates": [],
        "initial_selected_item_ids": [],
        "output_root": "fixture-output-root",
        "layout": "sentinel1_slc"
    }))
    .expect("V2 snapshot remains readable");
    assert!(serde_json::to_value(legacy)
        .unwrap()
        .get("transfer_items")
        .is_none());
}

#[test]
fn generated_orbit_contracts_validate_windows_and_basenames() {
    let valid = OrbitFileMetadata {
        file_name: "S1A_OPER_AUX_POEORB_OPOD_20240102T120000_V20240101T225942_20240103T005942.EOF"
            .to_string(),
        platform: OrbitPlatform::S1A,
        orbit_type: OrbitProductType::POEORB,
        creation_unix_seconds: 1_704_196_800,
        validity_start_unix_seconds: 1_704_150_000,
        validity_stop_unix_seconds: 1_704_243_600,
    };
    assert!(valid.validate().is_ok());

    let mut invalid_window = valid.clone();
    invalid_window.validity_stop_unix_seconds = invalid_window.validity_start_unix_seconds;
    assert_eq!(
        invalid_window.validate().unwrap_err(),
        "validity stop must be after start"
    );

    let mut invalid_file_name = valid.clone();
    invalid_file_name.file_name = "nested/orbit.EOF".to_string();
    assert_eq!(
        invalid_file_name.validate().unwrap_err(),
        "file_name must be a basename"
    );

    for file_name in [
        "",
        "nested/file.EOF",
        r"C:\orbits\file.EOF",
        "bad\0name.EOF",
    ] {
        let request = OrbitParseRequest {
            file_names: vec![file_name.to_string()],
        };
        assert!(request.validate().unwrap_err().contains("basename"));
    }
}

#[test]
fn generated_contracts_validate_constraints_and_sensitive_context() {
    let invalid_bbox = BBox {
        west: 111.0,
        east: 110.0,
        south: 30.0,
        north: 31.0,
        crs: "EPSG:4326".to_string(),
    };
    assert!(invalid_bbox.validate().is_err());

    let mut query: ProviderQuery =
        serde_json::from_str(&fs::read_to_string(fixture_path("provider_query.json")).unwrap())
            .unwrap();
    query.max_results = Some(0);
    assert!(query.validate().is_err());

    let mut sensitive = serde_json::Map::new();
    sensitive.insert(
        "nested".to_string(),
        serde_json::json!([{"Authorization": "not-allowed"}]),
    );
    let error = ContractError {
        code: "V3_SECRET".to_string(),
        user_message: "敏感上下文测试".to_string(),
        debug_context: sensitive,
    };
    assert!(error.validate().is_err());
}

#[test]
fn search_diagnostics_validate_complete_count_semantics() {
    let valid: AsfSearchStats = serde_json::from_value(serde_json::json!({
        "requested_limit": 5,
        "query_limit": 20,
        "total_count": 20,
        "candidate_count": 20,
        "returned_count": 5,
        "source": "ASF_BATCH",
        "cmr_enriched": null,
        "matched_count": 20,
        "fetched_record_count": 9,
        "replayed_record_count": 1,
        "mapped_record_count": 7,
        "mapping_error_count": 1,
        "aggregated_asset_record_count": 8,
        "unique_provider_product_count": 6,
        "asset_count": 7,
        "identity_conflict_count": 1,
        "page_size": 250,
        "truncated": true,
        "item_errors": [{
            "code": "ASF_METADATA_INVALID",
            "user_message": "Fixture mapping error",
            "debug_context": {"reason": "missing_product_id"}
        }],
        "warnings": []
    }))
    .expect("new diagnostic fields deserialize");
    valid.validate().unwrap();

    let mut invalid_returned = valid.clone();
    invalid_returned.returned_count = 7;
    assert_eq!(
        invalid_returned.validate().unwrap_err(),
        "returned_count must not exceed unique_provider_product_count"
    );

    let mut invalid_unique = valid;
    invalid_unique.unique_provider_product_count = Some(8);
    assert_eq!(
        invalid_unique.validate().unwrap_err(),
        "unique_provider_product_count must not exceed mapped_record_count"
    );
}

#[test]
fn agent_tool_round_host_facts_round_trip_and_default_when_absent() {
    let legacy: insar_core::AgentToolRoundPayload = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "call_id": "c1", "tool_id": "batch_page", "status": "running"
    }))
    .unwrap();
    assert_eq!(legacy.started_at, None);
    assert_eq!(legacy.duration_ms, None);
    assert_eq!(legacy.result_count, None);
    assert_eq!(legacy.summary_zh, None);
    assert_eq!(legacy.target_label, None);
    assert_eq!(legacy.input_summary, None);
    assert_eq!(legacy.output_summary, None);
    let full = serde_json::json!({
        "run_id": "run-1", "call_id": "c1", "tool_id": "search_progress", "status": "done",
        "started_at": "2026-10-25T08:00:00Z", "duration_ms": 12, "result_count": 3,
        "summary_zh": "检索完成，保留 3 景",
        "target_label": "sentinel1 · 合肥市",
        "input_summary": "{\"logical_dataset\":\"sentinel1\"}",
        "output_summary": "{\"items_kept\":3}"
    });
    let round: insar_core::AgentToolRoundPayload = serde_json::from_value(full.clone()).unwrap();
    assert!(round.validate().is_ok());
    assert_eq!(round.result_count, Some(3));
    assert_eq!(serde_json::to_value(&round).unwrap(), full);
}

#[test]
fn agent_card_snapshot_counts_and_orbit_directions_default_when_absent() {
    let range = serde_json::json!({
        "aoi_name": "西宁北", "area_km2": null, "time_range": null, "product": "3 个产品"
    });
    let legacy: insar_core::AgentCardSnapshot = serde_json::from_value(serde_json::json!({
        "source": "asf.sentinel1", "range": range, "estimated_bytes": null, "output_dir": null
    }))
    .unwrap();
    assert_eq!(legacy.result_count, None);
    assert_eq!(legacy.already_queued_count, None);
    assert!(legacy.orbit_directions.is_empty());
    let full = serde_json::json!({
        "source": "asf.sentinel1", "range": range, "estimated_bytes": null, "output_dir": null,
        "output_dir_source": null, "result_count": 3, "already_queued_count": 1,
        "orbit_directions": ["Ascending", "Descending"],
        "source_display_name_zh": null,
        "source_resource_id": null,
        "selected_items_preview": []
    });
    let card: insar_core::AgentCardSnapshot = serde_json::from_value(full.clone()).unwrap();
    assert!(card.validate().is_ok());
    assert_eq!(serde_json::to_value(&card).unwrap(), full);
}

#[test]
fn agent_claim_ref_run_record_step_round_trips_and_defaults_when_absent() {
    let legacy: insar_core::AgentClaimRef = serde_json::from_value(serde_json::json!({
        "kind": "web", "id": "https://example.org/x", "label": "https://example.org/x"
    }))
    .unwrap();
    assert_eq!(legacy.trace_sequence, None);
    assert_eq!(legacy.response_count, None);
    let full = serde_json::json!({
        "kind": "batch", "id": "batch-b", "label": "批次 batch-b",
        "trace_sequence": 3, "response_count": 3
    });
    let reference: insar_core::AgentClaimRef = serde_json::from_value(full.clone()).unwrap();
    assert!(reference.validate().is_ok());
    assert_eq!(serde_json::to_value(&reference).unwrap(), full);
}

#[test]
fn agent_permission_mode_is_three_plain_names_without_limits() {
    for name in ["confirm_changes", "auto_approve", "full_access"] {
        let mode: insar_core::AgentPermissionMode =
            serde_json::from_value(serde_json::json!(name)).unwrap();
        assert_eq!(serde_json::to_value(mode).unwrap(), serde_json::json!(name));
    }
    for old in [
        serde_json::json!("per_item"),
        serde_json::json!("within_quota"),
        serde_json::json!({"kind": "full_access"}),
    ] {
        assert!(serde_json::from_value::<insar_core::AgentPermissionMode>(old).is_err());
    }
    let view: insar_core::AgentPermissionModeView = serde_json::from_value(serde_json::json!({
        "effective": "auto_approve", "session_override": null, "global_default": "auto_approve"
    }))
    .unwrap();
    assert!(view.session_override.is_none());
    let stop = serde_json::json!({
        "cancelled_runs": ["run-1"], "failed_runs": [], "cancelled_tasks": ["task-1"],
        "failed_tasks": []
    });
    let result: insar_core::AgentStopAllResult = serde_json::from_value(stop.clone()).unwrap();
    assert!(result.validate().is_ok());
    assert_eq!(serde_json::to_value(&result).unwrap(), stop);
}

#[test]
fn agent_card_output_dir_source_and_search_disclosure_default_when_absent() {
    let legacy: insar_core::AgentCardSnapshot = serde_json::from_value(serde_json::json!({
        "source": "asf.sentinel1", "range": {"aoi_name": "a", "area_km2": null,
            "time_range": null, "product": "1 个产品"},
        "estimated_bytes": null, "output_dir": "D:/out"
    }))
    .unwrap();
    assert!(legacy.output_dir_source.is_none());
    for source in [
        "model_suggested",
        "default",
        "chosen_on_card",
        "from_message",
    ] {
        let card: insar_core::AgentCardSnapshot = serde_json::from_value(serde_json::json!({
            "source": "asf.sentinel1", "range": {"aoi_name": "a", "area_km2": null,
                "time_range": null, "product": "1 个产品"},
            "estimated_bytes": 1, "output_dir": "D:/out", "output_dir_source": source
        }))
        .unwrap();
        assert!(card.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&card).unwrap()["output_dir_source"],
            serde_json::json!(source)
        );
    }
    assert!(
        serde_json::from_value::<insar_core::AgentCardSnapshot>(serde_json::json!({
            "source": "s", "range": {"aoi_name": "a", "area_km2": null, "time_range": null,
                "product": "p"},
            "estimated_bytes": null, "output_dir": null, "output_dir_source": "quota"
        }))
        .is_err()
    );
    let step: insar_core::AgentPlanCardStep = serde_json::from_value(serde_json::json!({
        "tool_id": "search_start", "label_zh": "检索", "tier": 1, "status": "pending",
        "disclosure": {"resource_id": "science.sentinel-1.slc.v1", "aoi_name": "区域",
            "time_range": null}
    }))
    .unwrap();
    assert!(step.validate().is_ok());
    assert!(step.disclosure.as_ref().unwrap().display_name_zh.is_none());
    let named: insar_core::AgentPlanCardStep = serde_json::from_value(serde_json::json!({
        "tool_id": "search_start", "label_zh": "检索", "tier": 1, "status": "pending",
        "disclosure": {"resource_id": "science.sentinel-1.slc.v1", "aoi_name": "区域",
            "time_range": null, "display_name_zh": "Sentinel-1 SLC"}
    }))
    .unwrap();
    assert!(named.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&named).unwrap()["disclosure"]["display_name_zh"],
        serde_json::json!("Sentinel-1 SLC")
    );
    let plain: insar_core::AgentPlanCardStep = serde_json::from_value(serde_json::json!({
        "tool_id": "create_task", "label_zh": "建任务", "tier": 2, "status": "pending"
    }))
    .unwrap();
    assert!(plain.disclosure.is_none());
}

#[test]
fn agent_card_approval_defaults_when_absent_and_round_trips() {
    let legacy: insar_core::AgentPlanCard = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "plan_id": "plan-1", "plan_hash": "a".repeat(64),
        "max_tier": 2, "steps": []
    }))
    .unwrap();
    assert!(legacy.approval.is_none());
    assert!(!legacy.auto_executed);
    let full = serde_json::json!({
        "run_id": "run-1", "plan_id": "plan-1", "plan_hash": "a".repeat(64),
        "max_tier": 2, "steps": [],
        "approval": {"decision": "manual", "mode": "auto_approve",
            "card_reasons": ["size_unknown", "targets_user_task"]},
        "auto_executed": false
    });
    let card: insar_core::AgentPlanCard = serde_json::from_value(full.clone()).unwrap();
    assert!(card.validate().is_ok());
    assert_eq!(serde_json::to_value(&card).unwrap(), full);
    let refused: insar_core::AgentErrorPayload = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "code": "plan_refused_forbidden", "status": null, "plan_id": null,
        "approval": {"decision": "refused_forbidden", "mode": "full_access", "card_reasons": []}
    }))
    .unwrap();
    assert!(refused.approval.is_some());
    assert!(serde_json::from_value::<insar_core::AgentCardApproval>(
        serde_json::json!({"decision": "auto", "mode": "full_access"})
    )
    .is_err());
    assert!(serde_json::from_value::<insar_core::AgentCardApproval>(
        serde_json::json!({"decision": "manual", "mode": "auto_approve", "card_reasons": ["max_bytes"]})
    )
    .is_err());
}

#[test]
fn agent_channel_reasoning_mode_reads_off_when_absent_and_keeps_saved_on_save() {
    let legacy: insar_core::AgentChannelView = serde_json::from_value(serde_json::json!({
        "id": "c1", "protocol": "chat_completions", "base_url": "https://example.invalid/v1",
        "model": "glm-5.3-flash", "key_ready": true, "is_default": true
    }))
    .unwrap();
    assert_eq!(legacy.reasoning_mode, insar_core::AgentReasoningMode::Off);
    // W3o: a host older than the remembered refusal reads as not refused.
    assert!(!legacy.reasoning_switch_unsupported);
    let remembered: insar_core::AgentChannelView = serde_json::from_value(serde_json::json!({
        "id": "c1", "protocol": "chat_completions", "base_url": "https://example.invalid/v1",
        "model": "glm-5.3-flash", "reasoning_mode": "off", "reasoning_switch_unsupported": true,
        "key_ready": true, "is_default": true
    }))
    .unwrap();
    assert!(remembered.reasoning_switch_unsupported);
    for name in ["default", "off"] {
        let mode: insar_core::AgentReasoningMode =
            serde_json::from_value(serde_json::json!(name)).unwrap();
        assert_eq!(serde_json::to_value(mode).unwrap(), serde_json::json!(name));
    }
    assert!(
        serde_json::from_value::<insar_core::AgentReasoningMode>(serde_json::json!("high"))
            .is_err()
    );
    let save: insar_core::AgentChannelSave = serde_json::from_value(serde_json::json!({
        "id": "c1", "protocol": "chat_completions", "base_url": "https://example.invalid/v1",
        "model": "glm-5.3-flash", "api_key": null
    }))
    .unwrap();
    assert!(save.reasoning_mode.is_none());
    let conclusion: insar_core::AgentConclusion = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "text": "", "claims": []
    }))
    .unwrap();
    assert!(!conclusion.reasoning_off_refused);
}

#[test]
fn agent_run_summary_flags_round_trip_and_default_to_false_when_absent() {
    let legacy: insar_core::AgentRunSummary = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "session_id": "session-1", "status": "completed",
        "started_at": "2026-10-26T00:00:00Z", "ended_at": null, "model_route": null
    }))
    .unwrap();
    assert!(!legacy.reasoning_off_refused && !legacy.truncated);
    let full = serde_json::json!({
        "run_id": "run-2", "session_id": "session-1", "status": "failed",
        "started_at": "2026-10-26T00:00:00Z", "ended_at": "2026-10-26T00:00:05Z",
        "model_route": "chat_completions/glm", "reasoning_off_refused": true, "truncated": true,
        "usage_total": null
    });
    let summary: insar_core::AgentRunSummary = serde_json::from_value(full.clone()).unwrap();
    assert!(summary.validate().is_ok());
    assert_eq!(serde_json::to_value(&summary).unwrap(), full);
}

#[test]
fn agent_activity_status_round_trips_and_is_a_stream_event_kind() {
    let status = serde_json::json!({
        "live_runs": 2, "agent_active_tasks": 1,
        "sessions_with_live_runs": ["session-a", "session-b"]
    });
    let parsed: insar_core::AgentActivityStatus = serde_json::from_value(status.clone()).unwrap();
    assert!(parsed.validate().is_ok());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), status);
    let kind: insar_core::AgentStreamEventKind =
        serde_json::from_value(serde_json::json!("activity_changed")).unwrap();
    assert_eq!(
        serde_json::to_value(kind).unwrap(),
        serde_json::json!("activity_changed")
    );
    assert!(
        serde_json::from_value::<insar_core::AgentActivityStatus>(serde_json::json!({
            "live_runs": 0, "agent_active_tasks": 0
        }))
        .is_err()
    );
}

#[test]
fn agent_error_payload_message_and_field_class_are_optional() {
    let legacy: insar_core::AgentErrorPayload = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "code": "model_capability", "status": "failed", "plan_id": null
    }))
    .unwrap();
    assert!(legacy.message_zh.is_none() && legacy.field_class.is_none());
    let blocked = serde_json::json!({
        "run_id": "run-2", "code": "outbound_blocked", "status": "failed", "plan_id": null,
        "approval": null, "message_zh": "没有发给模型服务", "field_class": "tool_result"
    });
    let payload: insar_core::AgentErrorPayload = serde_json::from_value(blocked.clone()).unwrap();
    assert!(payload.validate().is_ok());
    assert_eq!(serde_json::to_value(&payload).unwrap(), blocked);
}

#[test]
fn agent_card_snapshot_source_display_name_is_optional() {
    let base = serde_json::json!({
        "source": "asf.catalog",
        "range": {"aoi_name": "叠溪", "area_km2": null, "time_range": null, "product": "1 个产品"},
        "estimated_bytes": 10, "output_dir": null
    });
    let legacy: insar_core::AgentCardSnapshot = serde_json::from_value(base.clone()).unwrap();
    assert!(legacy.source_display_name_zh.is_none());
    let mut named = base;
    named["source_display_name_zh"] = serde_json::json!("Sentinel-1 标准产品");
    let snapshot: insar_core::AgentCardSnapshot = serde_json::from_value(named).unwrap();
    assert!(snapshot.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap()["source_display_name_zh"],
        "Sentinel-1 标准产品"
    );
}

#[test]
fn agent_run_summary_usage_total_is_optional() {
    let base = serde_json::json!({
        "run_id": "run-1", "session_id": "session-1", "status": "completed",
        "started_at": "2026-10-27T00:00:00Z", "ended_at": null, "model_route": null
    });
    let legacy: insar_core::AgentRunSummary = serde_json::from_value(base.clone()).unwrap();
    assert!(legacy.usage_total.is_none());
    let mut full = base;
    full["usage_total"] = serde_json::json!({"prompt": 220, "completion": 40, "total": 260});
    let summary: insar_core::AgentRunSummary = serde_json::from_value(full).unwrap();
    assert!(summary.validate().is_ok());
    assert_eq!(
        serde_json::to_value(&summary).unwrap()["usage_total"],
        serde_json::json!({"prompt": 220, "completion": 40, "total": 260})
    );
}

#[test]
fn agent_20_thinking_question_workspace_sync_and_aoi_set_contracts_round_trip() {
    let legacy_msg: insar_core::AgentMessagePayload = serde_json::from_value(serde_json::json!({
        "role": "assistant", "run_id": "run-1", "text": "您好", "context": null
    }))
    .unwrap();
    assert!(legacy_msg.thinking_text.is_none() && legacy_msg.thinking_ms.is_none());

    let legacy_conclusion: insar_core::AgentConclusion =
        serde_json::from_value(serde_json::json!({
            "run_id": "run-1", "text": "### 结果", "claims": []
        }))
        .unwrap();
    assert!(legacy_conclusion.thinking_text.is_none() && legacy_conclusion.thinking_ms.is_none());

    let aoi_in: insar_core::AgentWorkingAoiSetInput = serde_json::from_value(serde_json::json!({
        "admin_code": "510000"
    }))
    .unwrap();
    assert!(aoi_in.validate().is_ok());
    assert_eq!(aoi_in.admin_code.as_deref(), Some("510000"));
    assert!(!aoi_in.clear);

    let thinking: insar_core::AgentThinkingPayload = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "step_index": 0, "status": "done", "text": "先确认四川省编码", "thinking_ms": 1840
    }))
    .unwrap();
    assert!(thinking.validate().is_ok());
    assert_eq!(thinking.thinking_ms, Some(1840));

    let question: insar_core::AgentQuestionCardPayload =
        serde_json::from_value(serde_json::json!({
            "run_id": "run-1",
            "question_zh": "请选择哨兵1号产品级别",
            "options": [{"label_zh": "SLC (干涉测量)", "value": "SLC", "recommended": true}]
        }))
        .unwrap();
    assert!(question.validate().is_ok());
    assert_eq!(question.options.len(), 1);

    let sync: insar_core::AgentWorkspaceSyncPayload = serde_json::from_value(serde_json::json!({
        "run_id": "run-1", "action": "open_panel", "tab": "map"
    }))
    .unwrap();
    assert!(sync.validate().is_ok());
}
