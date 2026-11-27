use insar_core::canonical_product_key;

#[test]
fn product_key_separates_provider_dataset_and_native_id() {
    let sentinel = canonical_product_key("asf", "sentinel-1", "same").unwrap();
    let ers = canonical_product_key("ASF", "ERS", "same").unwrap();

    assert_eq!(sentinel, "ASF::SENTINEL-1::same");
    assert_eq!(ers, "ASF::ERS::same");
    assert_ne!(sentinel, ers);
}

#[test]
fn product_key_escapes_ambiguous_native_delimiters() {
    assert_eq!(
        canonical_product_key("ASF", "ERS", "A::B%1").unwrap(),
        "ASF::ERS::A%3A%3AB%251"
    );
}

#[test]
fn product_key_rejects_missing_identity_components() {
    for (provider, dataset, product_id) in [
        ("", "ERS", "A"),
        ("ASF", "", "A"),
        ("ASF", "ERS", ""),
        ("  ", "ERS", "A"),
    ] {
        assert!(canonical_product_key(provider, dataset, product_id).is_err());
    }
}
