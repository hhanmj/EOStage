use insar_core::{provider_product_key, stable_asset_key, StableAssetReference};

#[test]
fn provider_and_asset_keys_preserve_opaque_native_identity() {
    let product = provider_product_key("asf.catalog", "ALOS PALSAR", "Item::Mixed%Case")
        .expect("complete Provider identity is valid");
    assert_eq!(
        product,
        "PROVIDER::asf.catalog::COLLECTION::ALOS PALSAR::ITEM::Item%3A%3AMixed%25Case"
    );

    let asset =
        stable_asset_key(&product, "File::L1.1%H5").expect("complete asset identity is valid");
    assert_eq!(
        asset,
        concat!(
            "PROVIDER::asf.catalog::COLLECTION::ALOS PALSAR::",
            "ITEM::Item%3A%3AMixed%25Case::ASSET::File%3A%3AL1.1%25H5"
        )
    );
}

#[test]
fn stable_asset_reference_round_trips_without_access_secrets() {
    let provider_product_key =
        provider_product_key("asf.catalog", "ALOS PALSAR", "ALOS-ITEM-001").unwrap();
    let stable_asset_key = stable_asset_key(&provider_product_key, "ALOS-FILE-001").unwrap();
    let reference = StableAssetReference {
        provider_id: "asf.catalog".to_string(),
        collection_id: "ALOS PALSAR".to_string(),
        provider_product_id: "ALOS-ITEM-001".to_string(),
        provider_product_key,
        canonical_product_key: None,
        asset_key: "ALOS-FILE-001".to_string(),
        stable_asset_key,
    };

    reference.validate().unwrap();
    let value = serde_json::to_value(&reference).unwrap();
    assert!(value.get("url").is_none());
    assert!(value.get("headers").is_none());
    assert!(value.get("cookies").is_none());
    assert!(value.get("token").is_none());
    let restored: StableAssetReference = serde_json::from_value(value).unwrap();
    assert_eq!(restored, reference);
}

#[test]
fn stable_identity_rejects_missing_components() {
    assert!(provider_product_key("", "ALOS PALSAR", "A").is_err());
    assert!(provider_product_key("asf.catalog", "", "A").is_err());
    assert!(provider_product_key("asf.catalog", "ALOS PALSAR", "").is_err());
    assert!(stable_asset_key("", "asset").is_err());
    assert!(stable_asset_key("provider-product", "").is_err());
}
