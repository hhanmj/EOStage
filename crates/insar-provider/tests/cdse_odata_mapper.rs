use std::fs;
use std::path::PathBuf;

use insar_core::{AssetRole, CatalogSearchQuery};
use insar_provider::catalog::RouteUse;
use insar_provider::cdse::odata_mapper::{
    map_odata_page, select_quicklook_descriptor, CDSE_ODATA_ASSET_VALUE_BASE,
};
use insar_provider::cdse::odata_types::{ODataAsset, ODataPage};
use insar_provider::cdse::query::{build_odata_request_for_use, Sentinel2SearchFacts};
use serde_json::json;

fn fixture(name: &str) -> ODataPage {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/catalog/cdse/sentinel2_l2a")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("fixture reads"))
        .expect("fixture has reviewed OData shape")
}

fn facts(with_cloud_filter: bool) -> Sentinel2SearchFacts {
    facts_for_use(with_cloud_filter, RouteUse::OfflineFixture)
}

fn production_facts(with_cloud_filter: bool) -> Sentinel2SearchFacts {
    facts_for_use(with_cloud_filter, RouteUse::Production)
}

#[test]
fn selected_spacecraft_rejects_out_of_scope_provider_records() {
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a", "provider_id": "copernicus.dataspace.odata", "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]},
        "cursor": null, "result_limit": 2,
        "filters": {"product_type":"S2MSI2A", "spacecraft":["sentinel_2c"]}
    })).unwrap();
    let request = build_odata_request_for_use(&query, RouteUse::Production).unwrap();
    let page: ODataPage = serde_json::from_value(json!({"value":[{
        "Id":"00000000-0000-4000-8000-000000000001",
        "Name":"S2A_MSIL2A_20260101T000000_N0511_R000_T00AAA_20260101T010000.SAFE",
        "Collection":{"Name":"SENTINEL-2"}, "ContentLength":2048,
        "ContentDate":{"Start":"2026-01-01T00:00:00Z"}, "Online":true,
        "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
    }]}))
    .unwrap();
    let response = map_odata_page(page, request.facts(), None).unwrap();
    assert!(response.products.is_empty());
    assert_eq!(response.item_errors.len(), 1);
    assert_eq!(
        response.item_errors[0].debug_context["reason"],
        "spacecraft_scope"
    );
}

fn l1c_facts() -> Sentinel2SearchFacts {
    let query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l1c", "provider_id": "copernicus.dataspace.odata", "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]},
        "cursor": null, "result_limit": 2,
        "filters": {"product_type":"S2MSI1C","datetime":{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"}}
    }))
    .expect("L1C query shape");
    build_odata_request_for_use(&query, RouteUse::OfflineFixture)
        .expect("L1C offline facts")
        .facts()
        .clone()
}

fn facts_for_use(with_cloud_filter: bool, use_case: RouteUse) -> Sentinel2SearchFacts {
    let mut query: CatalogSearchQuery = serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a", "provider_id": "copernicus.dataspace.odata", "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]},
        "cursor": null, "result_limit": 2,
        "filters": {"product_type":"S2MSI2A","cloud_cover_lte":4.2,"datetime":{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"}}
    }))
    .expect("query shape");
    if !with_cloud_filter {
        query.filters.remove("cloud_cover_lte");
    }
    build_odata_request_for_use(&query, use_case)
        .expect("query facts")
        .facts()
        .clone()
}

#[test]
fn authoritative_odata_records_map_to_one_non_executable_archive_per_uuid() {
    let response = map_odata_page(fixture("odata_collection_page_1.json"), &facts(true), None)
        .expect("authoritative OData page maps");
    assert_eq!(
        response.collection.provider_id,
        "copernicus.dataspace.odata"
    );
    assert_eq!(response.products.len(), 2);
    let first = &response.products[0];
    assert_eq!(first.product_id, "00000000-0000-4000-8000-000000000001");
    assert_eq!(first.assets[0].asset_key.as_deref(), Some("archive"));
    assert_eq!(first.assets[0].native_roles, ["archive"]);
    assert!(first.assets[0].url.is_none());
    assert!(first.assets[0].access_descriptor.is_none());
    assert!(first.assets[0].file_name.is_none());
    assert_eq!(first.properties["cloud_cover"], 4.2);
    assert!(response.collection.production_enabled);
}

#[test]
fn sentinel2_l1c_records_map_only_into_the_default_closed_l1c_collection() {
    let l1c_page: ODataPage = serde_json::from_value(json!({
        "value": [{
            "Id":"00000000-0000-4000-8000-0000000001c1",
            "Name":"S2A_MSIL1C_20260101T000000_N0511_R000_T00AAA_20260101T010000.SAFE",
            "Collection":{"Name":"SENTINEL-2"},
            "ContentLength":2048,
            "ContentDate":{"Start":"2026-01-01T00:00:00Z"},
            "Online":true,
            "Attributes":[{"Name":"productType","Value":"S2MSI1C"}]
        }]
    }))
    .expect("L1C page shape");
    let response =
        map_odata_page(l1c_page.clone(), &l1c_facts(), None).expect("L1C page maps offline");
    assert_eq!(response.products.len(), 1);
    assert_eq!(
        response.collection.logical_dataset,
        "optical.sentinel-2.l1c"
    );
    assert_eq!(
        response.collection.collection_profile_id,
        "cdse.sentinel-2.l1c.collection.v1"
    );
    assert_eq!(
        response.collection.identity_rule_id,
        "cdse.sentinel-2.l1c.identity.v1"
    );
    assert_eq!(
        response.collection.filename_rule_id,
        "cdse.sentinel-2.l1c.archive-filename.v1"
    );
    assert!(response.collection.production_enabled);
    assert_eq!(response.products[0].properties["product_type"], "S2MSI1C");

    let cross_profile =
        map_odata_page(l1c_page, &facts(false), None).expect("cross-profile record is isolated");
    assert!(cross_profile.products.is_empty());
    assert_eq!(cross_profile.item_errors.len(), 1);

    let l2a_record: ODataPage = serde_json::from_value(json!({
        "value": [{
            "Id":"00000000-0000-4000-8000-0000000002a1",
            "Name":"S2A_MSIL2A_20260101T000000_N0511_R000_T00AAA_20260101T010000.SAFE",
            "Collection":{"Name":"SENTINEL-2"},
            "ContentLength":2048,
            "ContentDate":{"Start":"2026-01-01T00:00:00Z"},
            "Online":true,
            "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
        }]
    }))
    .expect("L2A page shape");
    let cross_profile =
        map_odata_page(l2a_record, &l1c_facts(), None).expect("cross-profile record is isolated");
    assert!(cross_profile.products.is_empty());
    assert_eq!(cross_profile.item_errors.len(), 1);
}

#[test]
fn production_collection_proof_accepts_missing_echo_but_rejects_conflicts() {
    let page: ODataPage = serde_json::from_value(json!({
        "value": [
            {
                "Id":"00000000-0000-4000-8000-000000000111",
                "Name":"S2_PRODUCTION_NO_COLLECTION.SAFE",
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000112",
                "Name":"S2_PRODUCTION_ECHOED_COLLECTION.SAFE",
                "Collection":{"Name":"SENTINEL-2"},
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000113",
                "Name":"S2_PRODUCTION_MALFORMED_COLLECTION.SAFE",
                "Collection":{},
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000114",
                "Name":"S2_PRODUCTION_CONFLICTING_COLLECTION.SAFE",
                "Collection":{"Name":"SENTINEL-1"},
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000115",
                "Name":"S2_PRODUCTION_MISSING_PRODUCT_TYPE.SAFE",
                "Collection":{"Name":"SENTINEL-2"},
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Attributes":[]
            }
        ]
    }))
    .expect("production collection proof fixture parses");

    let production = map_odata_page(page.clone(), &production_facts(false), None)
        .expect("invalid production records stay item-local");
    assert_eq!(production.products.len(), 2);
    assert_eq!(production.item_errors.len(), 3);
    assert_eq!(
        production
            .products
            .iter()
            .map(|product| product.product_id.as_str())
            .collect::<Vec<_>>(),
        [
            "00000000-0000-4000-8000-000000000111",
            "00000000-0000-4000-8000-000000000112",
        ]
    );

    let fixture =
        map_odata_page(page, &facts(false), None).expect("fixture records remain item-local");
    assert_eq!(fixture.products.len(), 1);
    assert_eq!(fixture.item_errors.len(), 4);
    assert_eq!(
        fixture.products[0].product_id,
        "00000000-0000-4000-8000-000000000112"
    );
}

#[test]
fn mapper_ignores_assets_in_bulk_search_response() {
    let asset_id = "10000000-0000-4000-8000-000000000001";
    let page: ODataPage = serde_json::from_value(json!({
        "value": [{
            "Id":"00000000-0000-4000-8000-000000000010",
            "Name":"S2_QUICKLOOK_FIXTURE.SAFE",
            "Collection":{"Name":"SENTINEL-2"},
            "ContentLength":2048,
            "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
            "Online":true,
            "Attributes":[{"Name":"productType","Value":"S2MSI2A"}],
            "Assets":[
                {
                    "Id":"10000000-0000-4000-8000-000000000002",
                    "Name":"S2_QUICKLOOK_FIXTURE_thumbnail.png",
                    "ContentType":"image/png",
                    "ContentLength":2048
                },
                {
                    "Id":asset_id,
                    "Name":"S2_QUICKLOOK_FIXTURE_quicklook.jpg",
                    "ContentType":"image/jpeg",
                    "ContentLength":4096,
                    "DownloadLink":format!("{CDSE_ODATA_ASSET_VALUE_BASE}({asset_id})/$value")
                }
            ]
        }]
    }))
    .expect("reviewed quicklook shape");

    let response = map_odata_page(page, &facts(false), None).expect("bulk product maps");
    let assets = &response.products[0].assets;
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].role, AssetRole::Primary);
    assert!(assets
        .iter()
        .all(|asset| asset.role != AssetRole::Thumbnail));
}

#[test]
fn lazy_quicklook_selection_is_closed_and_deterministic() {
    let asset_id = "10000000-0000-4000-8000-000000000001";
    let assets: Vec<ODataAsset> = serde_json::from_value(json!([
        {
            "Id":"10000000-0000-4000-8000-000000000002",
            "Name":"S2_thumbnail.png",
            "ContentType":"image/png",
            "ContentLength":2048
        },
        {
            "Id":asset_id,
            "Name":"S2_quicklook.jpg",
            "ContentType":"image/jpeg",
            "ContentLength":4096,
            "DownloadLink":format!("{CDSE_ODATA_ASSET_VALUE_BASE}({asset_id})/$value")
        }
    ]))
    .expect("reviewed assets parse");
    let selected = select_quicklook_descriptor(&assets)
        .expect("reviewed quicklook selects")
        .expect("one quicklook exists");
    assert_eq!(selected.asset_id, asset_id);
    assert_eq!(selected.media_type, "image/jpeg");
    assert_eq!(selected.expected_length, Some(4096));

    let live_shape: Vec<ODataAsset> = serde_json::from_value(json!([{
        "Type":"QUICKLOOK",
        "Id":"e054ab3f-a5df-4aa0-894b-fba293475180",
        "DownloadLink":"https://catalogue.dataspace.copernicus.eu/odata/v1/Assets(e054ab3f-a5df-4aa0-894b-fba293475180)/$value",
        "S3Path":"/eodata/Sentinel-2/MSI/L2A/example.SAFE"
    }]))
    .expect("current CDSE QUICKLOOK shape parses");
    let live_selected = select_quicklook_descriptor(&live_shape)
        .expect("current CDSE QUICKLOOK shape is reviewed")
        .expect("typed quicklook exists");
    assert_eq!(
        live_selected.asset_id,
        "e054ab3f-a5df-4aa0-894b-fba293475180"
    );
    assert_eq!(live_selected.media_type, "image/*");
    assert_eq!(live_selected.expected_length, None);

    let mut duplicate = assets.clone();
    duplicate.push(assets[1].clone());
    assert_eq!(
        select_quicklook_descriptor(&duplicate).expect("exact duplicate collapses"),
        Some(selected.clone())
    );

    for malformed in [
        json!([{
            "Id":"10000000-0000-4000-8000-000000000003",
            "Name":"S2_quicklook.jpg",
            "ContentType":"image/jpeg",
            "DownloadLink":"http://untrusted.example/asset.jpg"
        }]),
        json!([{
            "Id":"10000000-0000-4000-8000-000000000004",
            "Name":"S2_quicklook.gif",
            "ContentType":"image/gif"
        }]),
        json!([{
            "Id":"10000000-0000-4000-8000-00000000000A",
            "Name":"S2_quicklook.jpg",
            "ContentType":"image/jpeg"
        }]),
        json!([
            {
                "Id":"10000000-0000-4000-8000-000000000005",
                "Name":"S2_quicklook.jpg",
                "ContentType":"image/jpeg",
                "ContentLength":4096
            },
            {
                "Id":"10000000-0000-4000-8000-000000000005",
                "Name":"S2_quicklook.jpg",
                "ContentType":"image/jpeg",
                "ContentLength":8192
            }
        ]),
    ] {
        let assets: Vec<ODataAsset> = serde_json::from_value(malformed).expect("asset shape");
        assert!(select_quicklook_descriptor(&assets).is_err());
    }

    assert_eq!(
        select_quicklook_descriptor(&[]).expect("missing assets are optional"),
        None
    );
}

#[test]
fn mapper_preserves_reviewed_wkt_fallback_and_count_semantics() {
    let response = map_odata_page(fixture("odata_collection_page_2.json"), &facts(true), None)
        .expect("reviewed POLYGON fallback maps");
    assert_eq!(
        response.products[0].footprint_geojson.as_ref().unwrap()["type"],
        "Polygon"
    );
    assert_eq!(response.stats.matched_count, Some(3));
    assert_eq!(response.stats.fetched_record_count, 1);
    assert_eq!(response.stats.mapped_record_count, 1);
}

#[test]
fn mapper_isolates_structural_items_and_applies_query_truth_to_every_record() {
    let malformed = map_odata_page(fixture("odata_malformed_product.json"), &facts(false), None)
        .expect("one bad shape leaves sibling available");
    assert_eq!(malformed.stats.fetched_record_count, 2);
    assert_eq!(malformed.stats.mapped_record_count, 1);
    assert_eq!(malformed.item_errors.len(), 1);
    assert_eq!(malformed.products.len(), 1);

    let missing_safe_suffix: ODataPage = serde_json::from_value(json!({
        "value": [{
            "Id":"00000000-0000-4000-8000-000000000098",
            "Name":"S2_NO_SAFE_SUFFIX",
            "Collection":{"Name":"SENTINEL-2"},
            "ContentLength":1,
            "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
            "Online":true,
            "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
        }]
    }))
    .expect("fixture shape");
    let missing_safe_suffix = map_odata_page(missing_safe_suffix, &facts(false), None)
        .expect("invalid product names become item errors");
    assert!(missing_safe_suffix.products.is_empty());
    assert_eq!(missing_safe_suffix.item_errors.len(), 1);

    let scoped = map_odata_page(fixture("odata_query_scope_errors.json"), &facts(true), None)
        .expect("out-of-scope records are item errors");
    assert_eq!(scoped.stats.fetched_record_count, 6);
    assert_eq!(scoped.stats.mapped_record_count, 0);
    assert_eq!(scoped.item_errors.len(), 6);
}

#[test]
fn mapper_exercises_optional_and_identity_fixtures_with_deterministic_checksum_rules() {
    let no_footprint = map_odata_page(fixture("odata_missing_footprint.json"), &facts(false), None)
        .expect("missing footprint is explicit truth");
    assert!(no_footprint.products[0].footprint_geojson.is_none());

    let no_size = map_odata_page(fixture("odata_missing_size.json"), &facts(false), None)
        .expect("missing size is explicit truth");
    assert!(no_size.products[0].assets[0].size_bytes.is_none());

    let checksums = map_odata_page(
        fixture("odata_multiple_checksums.json"),
        &facts(false),
        None,
    )
    .expect("multiple checksums map deterministically");
    assert_eq!(
        checksums.products[0].properties["checksums"],
        json!([
            {"algorithm":"MD5","value":"00000000000000000000000000000000"},
            {"algorithm":"SHA256","value":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        ])
    );

    let conflicting = map_odata_page(
        fixture("odata_conflicting_checksums.json"),
        &facts(false),
        None,
    )
    .expect("checksum conflict becomes one item error");
    assert_eq!(conflicting.item_errors.len(), 1);

    let similar = map_odata_page(
        fixture("odata_similar_name_distinct_uuid.json"),
        &facts(false),
        None,
    )
    .expect("similar names do not deduplicate");
    assert_eq!(similar.products.len(), 2);

    let unavailable = map_odata_page(fixture("odata_unavailable.json"), &facts(false), None)
        .expect("offline product is catalogued but non-executable");
    assert_eq!(unavailable.products[0].properties["online"], false);
    assert!(unavailable.products[0].assets[0]
        .access_descriptor
        .is_none());
}

#[test]
fn mapper_retains_only_supported_checksum_evidence() {
    let page: ODataPage = serde_json::from_value(json!({
        "value": [
            {
                "Id":"00000000-0000-4000-8000-000000000121",
                "Name":"S2_SUPPORTED_CHECKSUMS.SAFE",
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Checksum":[
                    {"Algorithm":"BLAKE3","Value":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
                    {"Algorithm":"SHA256","Value":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
                    {"Algorithm":"MD5","Value":"00000000000000000000000000000000"}
                ],
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000122",
                "Name":"S2_SUPPLEMENTARY_ONLY.SAFE",
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Checksum":[{"Algorithm":"BLAKE3","Value":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}],
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            },
            {
                "Id":"00000000-0000-4000-8000-000000000123",
                "Name":"S2_INVALID_MD5.SAFE",
                "ContentLength":1,
                "ContentDate":{"Start":"2026-01-01T10:00:00Z"},
                "Online":true,
                "Checksum":[{"Algorithm":"MD5","Value":"not-a-checksum"}],
                "Attributes":[{"Name":"productType","Value":"S2MSI2A"}]
            }
        ]
    }))
    .expect("synthetic checksum page parses");

    let response = map_odata_page(page, &production_facts(false), None)
        .expect("unsupported supplementary checksums stay item-local");
    assert_eq!(response.products.len(), 2);
    assert_eq!(response.item_errors.len(), 1);
    assert_eq!(
        response.products[0].properties["checksums"],
        json!([
            {"algorithm":"MD5","value":"00000000000000000000000000000000"},
            {"algorithm":"SHA256","value":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        ])
    );
    assert!(
        response.products[1].properties.get("checksums").is_none(),
        "a BLAKE3-only record has no executable checksum evidence"
    );
}

#[test]
fn exact_duplicates_collapse_but_authoritative_conflicts_are_item_errors() {
    let duplicate = map_odata_page(fixture("odata_duplicate_exact.json"), &facts(false), None)
        .expect("exact duplicates map");
    assert_eq!(duplicate.stats.fetched_record_count, 2);
    assert_eq!(duplicate.stats.mapped_record_count, 2);
    assert_eq!(duplicate.stats.exact_duplicate_record_count, 1);
    assert_eq!(duplicate.stats.unique_provider_product_count, 1);
    assert_eq!(duplicate.products.len(), 1);

    let conflict: ODataPage = serde_json::from_value(json!({
        "value": [
            {"Id":"00000000-0000-4000-8000-000000000099","Name":"S2_A.SAFE","Collection":{"Name":"SENTINEL-2"},"ContentLength":1,"ContentDate":{"Start":"2026-01-01T10:00:00Z"},"Online":true,"Attributes":[{"Name":"productType","Value":"S2MSI2A"}]},
            {"Id":"00000000-0000-4000-8000-000000000099","Name":"S2_B.SAFE","Collection":{"Name":"SENTINEL-2"},"ContentLength":1,"ContentDate":{"Start":"2026-01-01T10:00:00Z"},"Online":true,"Attributes":[{"Name":"productType","Value":"S2MSI2A"}]}
        ]
    }))
    .unwrap();
    let conflict =
        map_odata_page(conflict, &facts(false), None).expect("conflict is per-record item errors");
    assert_eq!(conflict.stats.fetched_record_count, 2);
    assert_eq!(conflict.stats.mapped_record_count, 0);
    assert_eq!(conflict.stats.unique_provider_product_count, 0);
    assert_eq!(conflict.item_errors.len(), 2);
    assert!(conflict.products.is_empty());
}
