#[path = "support/mock_catalog.rs"]
mod mock_catalog;

use std::path::PathBuf;

use insar_core::{CatalogProvider, CatalogSearchQuery, ProviderRegistry};
use mock_catalog::FixtureCatalogProvider;
use serde_json::json;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/catalog/mock")
}

fn query(cursor: Option<&str>) -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "fixture.catalog",
        "native_collection_id": "FIXTURE-S2-L2A",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "cursor": cursor,
        "result_limit": 2,
        "filters": {}
    }))
    .expect("fixture query shape")
}

#[test]
fn fixture_only_catalog_provider_preserves_identity_counts_and_trusted_paging() {
    let provider = FixtureCatalogProvider::from_fixture_root(fixture_root());
    let collections = provider.collections().expect("collections read locally");
    assert_eq!(collections.len(), 1);
    assert!(!collections[0].production_enabled);

    let first = provider
        .search_catalog(&query(None))
        .expect("first page reads locally");
    assert_eq!(first.stats.matched_count, Some(3));
    assert_eq!(first.stats.fetched_record_count, 2);
    assert_eq!(first.stats.mapped_record_count, 2);
    assert_eq!(first.stats.exact_duplicate_record_count, 0);
    assert_eq!(first.stats.unique_provider_product_count, 2);
    assert_eq!(first.stats.returned_count, 2);
    assert_eq!(first.next_cursor.as_deref(), Some("fixture-page-2"));
    assert_eq!(first.products[0].provider_id, "fixture.catalog");
    assert_eq!(first.products[0].product_id, "fixture-item-1");
    assert_ne!(
        first.products[0].assets[0].stable_asset_key,
        first.products[1].assets[0].stable_asset_key
    );

    let second = provider
        .search_catalog(&query(Some("fixture-page-2")))
        .expect("trusted continuation reads locally");
    assert_eq!(second.stats.returned_count, 1);
    assert!(!second.stats.truncated);
    assert!(second.next_cursor.is_none());

    let deduplicated = provider
        .search_catalog(&query(Some("fixture-deduplicated")))
        .expect("deduplicated page reads locally");
    assert_eq!(deduplicated.stats.fetched_record_count, 3);
    assert_eq!(deduplicated.stats.mapped_record_count, 3);
    assert_eq!(deduplicated.stats.exact_duplicate_record_count, 1);
    assert_eq!(deduplicated.stats.unique_provider_product_count, 2);
    assert_eq!(deduplicated.stats.returned_count, 2);
    assert_eq!(deduplicated.products.len(), 2);

    let untrusted = provider
        .search_catalog(&query(Some("https://untrusted.example.invalid/page")))
        .expect_err("untrusted cursor is rejected");
    assert_eq!(untrusted.code, "CATALOG_CURSOR_UNTRUSTED");
}

#[test]
fn fixture_only_catalog_provider_classifies_malformed_and_never_promotes_preview() {
    let provider = FixtureCatalogProvider::from_fixture_root(fixture_root());
    let first = provider
        .search_catalog(&query(None))
        .expect("first page reads locally");
    let preview_only = &first.products[1];
    assert!(preview_only
        .assets
        .iter()
        .all(|asset| asset.role != insar_core::AssetRole::Primary));

    let malformed = provider
        .search_catalog(&query(Some("fixture-malformed")))
        .expect_err("malformed local record is classified");
    assert_eq!(malformed.code, "CATALOG_ITEM_MALFORMED");
    assert!(ProviderRegistry::new().metadata().is_empty());
}

#[test]
fn fixture_only_catalog_provider_fails_closed_for_catalogue_identity_conflicts() {
    let provider = FixtureCatalogProvider::from_fixture_root(fixture_root());

    for cursor in [
        "fixture-duplicate-product",
        "fixture-cross-provider",
        "fixture-duplicate-asset-key",
        "fixture-invalid-native-role",
    ] {
        assert_eq!(
            provider
                .search_catalog(&query(Some(cursor)))
                .unwrap_err()
                .code,
            "CATALOG_ITEM_MALFORMED",
            "{cursor} must not be accepted from fixed fixture evidence",
        );
    }
}

#[test]
fn fixture_only_catalog_provider_rejects_non_polygon_aoi_before_fixture_work() {
    let provider = FixtureCatalogProvider::from_fixture_root(fixture_root());
    let invalid: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "fixture.catalog",
        "native_collection_id": "FIXTURE-S2-L2A",
        "aoi_geojson": {"type": "LineString", "coordinates": [[110.0, 30.0], [111.0, 31.0]]},
        "cursor": null,
        "result_limit": 1,
        "filters": {}
    }))
    .expect("the generic shape parses before contract validation");

    assert_eq!(
        provider.search_catalog(&invalid).unwrap_err().code,
        "CATALOG_QUERY_INVALID"
    );
}
