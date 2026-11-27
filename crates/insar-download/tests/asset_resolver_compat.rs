use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole, ProviderAsset,
};
use insar_download::{classify_asset_access, AssetAccessClassification};

fn provider_asset(
    access_descriptor: Option<AssetAccessDescriptor>,
    url: Option<&str>,
) -> ProviderAsset {
    ProviderAsset {
        asset_key: Some("primary".to_string()),
        stable_asset_key: Some("fixture::product::primary".to_string()),
        access_descriptor,
        role: AssetRole::Primary,
        native_roles: Vec::new(),
        url: url.map(str::to_string),
        file_name: Some("fixture.bin".to_string()),
        media_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(1024),
    }
}

fn descriptor(resolver_id: &str) -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: resolver_id.to_string(),
        scheme: AssetAccessScheme::PublicHttps,
        stable_href: Some("https://example.org/fixture.bin".to_string()),
        requires_credentials: false,
        credential_profile: None,
        resolver_parameters: AssetResolverParameters {},
    }
}

#[test]
fn old_url_only_asset_is_an_explicit_legacy_adapter() {
    let asset = provider_asset(
        None,
        Some("https://legacy.example.org/file.bin?Signature=LEGACY_SECRET"),
    );
    let classified = classify_asset_access(&asset).unwrap();
    let AssetAccessClassification::LegacyAdapter(legacy) = classified else {
        panic!("URL-only compatibility asset was not classified as legacy")
    };
    assert_eq!(
        legacy.url(),
        "https://legacy.example.org/file.bin?Signature=LEGACY_SECRET"
    );
    assert!(!format!("{legacy:?}").contains("LEGACY_SECRET"));
}

#[test]
fn descriptor_is_authoritative_and_never_silently_falls_back() {
    let valid = descriptor("public_https");
    let asset = provider_asset(
        Some(valid.clone()),
        Some("https://legacy.example.org/file.bin"),
    );
    let AssetAccessClassification::Durable(actual) = classify_asset_access(&asset).unwrap() else {
        panic!("descriptor did not remain authoritative")
    };
    assert_eq!(actual, &valid);

    let invalid = provider_asset(
        Some(descriptor("")),
        Some("https://legacy.example.org/file.bin"),
    );
    assert_eq!(
        classify_asset_access(&invalid).unwrap_err().code(),
        "ASSET_ACCESS_DESCRIPTOR_INVALID"
    );
}

#[test]
fn new_descriptor_cannot_retain_a_secret_legacy_fallback_url() {
    for fallback in [
        "https://legacy.example.org/file.bin?Signature=SECRET",
        "https://user:password@legacy.example.org/file.bin",
        "https://legacy.example.org/file.bin#token",
    ] {
        let asset = provider_asset(Some(descriptor("public_https")), Some(fallback));
        assert_eq!(
            classify_asset_access(&asset).unwrap_err().code(),
            "ASSET_ACCESS_NOT_DURABLE",
            "secret fallback accepted: {fallback}"
        );
    }
}

#[test]
fn missing_descriptor_and_url_is_a_typed_invalid_reference() {
    let asset = provider_asset(None, None);
    assert_eq!(
        classify_asset_access(&asset).unwrap_err().code(),
        "ASSET_REFERENCE_INVALID"
    );
}
