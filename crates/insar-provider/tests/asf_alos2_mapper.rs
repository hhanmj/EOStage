use std::{fs, path::PathBuf};

use insar_core::{provider_product_key, AssetAccessScheme, AssetRole, DataSourceKind, ProductKind};
use insar_provider::{
    asf::{map_alos2_umm_granule, CmrGranuleEnvelope},
    catalog::{
        CollectionProfileRegistry, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    },
};
use serde_json::{json, Value};

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("ALOS-2 fixture exists"))
        .expect("ALOS-2 fixture is valid JSON")
}

fn base_wbd() -> Value {
    fixture("granule-wbd-valid.json")
}

fn map(value: &Value) -> Result<insar_core::ProviderProduct, insar_core::ContractError> {
    let envelope = CmrGranuleEnvelope::try_from(value)?;
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");
    map_alos2_umm_granule(&envelope, profile)
}

fn apply_mutation(base: &mut Value, mutation_fixture: &str) {
    let mutation = fixture(mutation_fixture);
    let mutation = mutation
        .get("mutation")
        .expect("mutation fixture has one mutation");
    let path = mutation["path"].as_str().expect("mutation path");
    *base.pointer_mut(path).expect("mutation target exists") = mutation["value"].clone();
}

#[test]
fn maps_authoritative_wbs_and_wbd_with_one_deduplicated_zip() {
    for name in ["granule-wbs-valid.json", "granule-wbd-valid.json"] {
        let source = fixture(name);
        let product = map(&source).expect("valid ALOS-2 fixture maps");
        let granule_id = source["granule"]["concept_id"].as_str().unwrap();
        let expected_key =
            provider_product_key(ALOS2_PROVIDER_ID, ALOS2_NATIVE_COLLECTION_ID, granule_id)
                .unwrap();

        assert_eq!(product.provider_id, ALOS2_PROVIDER_ID);
        assert_eq!(product.product_id, granule_id);
        assert_eq!(product.source_kind, DataSourceKind::Sar);
        assert_eq!(product.product_kind, ProductKind::SarProduct);
        assert_eq!(product.properties["provider_product_key"], expected_key);
        assert_eq!(
            product.properties["collection_concept_id"],
            ALOS2_NATIVE_COLLECTION_ID
        );
        assert_eq!(product.assets.len(), 1);
        let primary = &product.assets[0];
        assert_eq!(primary.role, AssetRole::Primary);
        assert_eq!(primary.native_roles, ["archive"]);
        assert_eq!(
            primary.file_name.as_deref(),
            source["granule"]["assets"][0]["file_name"].as_str()
        );
        assert_eq!(
            primary.size_bytes,
            source["granule"]["assets"][0]["size_bytes"].as_i64()
        );
        let descriptor = primary
            .access_descriptor
            .as_ref()
            .expect("HTTPS route is executable through the Resolver");
        assert_eq!(descriptor.scheme, AssetAccessScheme::EarthdataHttps);
        assert_eq!(descriptor.resolver_id, "earthdata_https");
        assert_eq!(
            descriptor.credential_profile.as_deref(),
            Some("earthdata-default")
        );
        let asset_key = primary.asset_key.as_deref().unwrap();
        assert_eq!(
            product.properties["asset_integrity"][asset_key]["algorithm"],
            "MD5"
        );
        assert_eq!(
            product.properties["asset_integrity"][asset_key]["value"],
            source["granule"]["assets"][0]["checksum"]["value"]
        );
        assert_eq!(
            product.properties["asset_routes"][asset_key]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(product.properties["preview_available"], false);
        assert!(product.payload.is_empty());
        product
            .validate()
            .expect("mapped product contract is valid");
    }
}

#[test]
fn rejects_collection_scope_and_missing_granule_identity_without_filename_fallback() {
    let mut historical = base_wbd();
    historical["collection"] = json!({
        "concept_id": "C1206485527-ASF",
        "short_name": "ALOS_PSR_L1.1",
        "version": "1"
    });
    assert_eq!(
        map(&historical).unwrap_err().code,
        "ASF_COLLECTION_IDENTITY_MISMATCH"
    );

    let mut missing_identity = base_wbd();
    missing_identity["granule"]["concept_id"] = Value::Null;
    assert_eq!(
        map(&missing_identity).unwrap_err().code,
        "ASF_PRODUCT_IDENTITY_MISSING"
    );

    for mutation in [
        "wrong-level.json",
        "wrong-mode.json",
        "wrong-polarization.json",
    ] {
        let mut value = base_wbd();
        apply_mutation(&mut value, mutation);
        assert_eq!(
            map(&value).unwrap_err().code,
            "ASF_PRODUCT_SCOPE_MISMATCH",
            "{mutation}"
        );
    }
}

#[test]
fn missing_ambiguous_and_malformed_primary_assets_fail_closed() {
    for (mutation, code) in [
        ("missing-primary-asset.json", "ASF_PRIMARY_ASSET_MISSING"),
        (
            "ambiguous-primary-assets.json",
            "ASF_PRIMARY_ASSET_AMBIGUOUS",
        ),
    ] {
        let mut value = base_wbd();
        apply_mutation(&mut value, mutation);
        assert_eq!(map(&value).unwrap_err().code, code, "{mutation}");
    }

    let mut unsafe_route = base_wbd();
    unsafe_route["granule"]["assets"][0]["routes"][0]["locator"] =
        json!("https://fixture-user@example.invalid/alos2/wbd.zip");
    assert_eq!(
        map(&unsafe_route).unwrap_err().code,
        "ASF_ASSET_CLASSIFICATION_INVALID"
    );
}

#[test]
fn no_browse_is_a_valid_product_state() {
    let product = map(&base_wbd()).expect("no Browse is not a product failure");
    assert_eq!(product.properties["preview_available"], false);
    assert!(product
        .assets
        .iter()
        .all(|asset| !matches!(asset.role, AssetRole::Browse | AssetRole::Thumbnail)));
}

#[test]
fn separate_https_and_s3_records_for_the_same_zip_deduplicate_deterministically() {
    let mut value = base_wbd();
    let original = value["granule"]["assets"][0].clone();
    let mut https = original.clone();
    https["routes"] = json!([original["routes"][0].clone()]);
    let mut s3 = original;
    s3["routes"] = json!([s3["routes"][1].clone()]);
    value["granule"]["assets"] = json!([s3, https]);

    let product = map(&value).expect("alternate locators identify one ZIP");
    assert_eq!(product.assets.len(), 1);
    let asset_key = product.assets[0].asset_key.as_deref().unwrap();
    let routes = product.properties["asset_routes"][asset_key]
        .as_array()
        .unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0]["scheme"], "https");
    assert_eq!(routes[1]["scheme"], "s3");
}
