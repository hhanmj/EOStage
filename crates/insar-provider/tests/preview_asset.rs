use insar_core::{
    AssetRole, PreviewAssetRole, PreviewCredentialRequirement, PreviewKind, ProviderAsset,
};
use insar_provider::preview::{resolve_preview_candidate, PreviewProductClass};

fn asset(role: AssetRole, key: &str, url: Option<&str>, media_type: &str) -> ProviderAsset {
    ProviderAsset {
        asset_key: Some(key.to_owned()),
        stable_asset_key: Some(format!("product::ASSET::{key}")),
        access_descriptor: None,
        role,
        native_roles: vec!["GET RELATED VISUALIZATION".to_owned()],
        url: url.map(str::to_owned),
        file_name: Some(format!("{key}.jpg")),
        media_type: Some(media_type.to_owned()),
        size_bytes: None,
    }
}

#[test]
fn sentinel_grd_selects_one_official_browse_without_using_primary_data() {
    let assets = vec![
        asset(
            AssetRole::Primary,
            "archive",
            Some("https://datapool.asf.alaska.edu/archive.zip"),
            "application/zip",
        ),
        asset(
            AssetRole::Browse,
            "browse",
            Some("https://datapool.asf.alaska.edu/browse.jpg"),
            "image/jpeg",
        ),
    ];

    let resolved = resolve_preview_candidate(
        "asf.catalog::SENTINEL-1::scene",
        PreviewProductClass::Sentinel1Grd,
        &assets,
    )
    .expect("one official ASF Browse is resolvable");

    assert_eq!(resolved.source_asset_role, PreviewAssetRole::Browse);
    assert_eq!(resolved.source_asset_key, "product::ASSET::browse");
    assert_eq!(resolved.descriptor.preview_kind, PreviewKind::Quicklook);
    assert_eq!(
        resolved.descriptor.credential_requirement,
        PreviewCredentialRequirement::None
    );
    assert_eq!(resolved.expected_media_type, "image/jpeg");
}

#[test]
fn nisar_requires_the_provider_classified_main_preview_and_never_falls_back() {
    let assets = vec![
        asset(
            AssetRole::Preview,
            "latlon-main",
            Some("https://nisar.asf.earthdatacloud.nasa.gov/main.png"),
            "image/png",
        ),
        asset(
            AssetRole::Browse,
            "other-browse",
            Some("https://nisar.asf.earthdatacloud.nasa.gov/other.png"),
            "image/png",
        ),
        asset(
            AssetRole::Thumbnail,
            "thumbnail",
            Some("https://nisar.asf.earthdatacloud.nasa.gov/thumb.png"),
            "image/png",
        ),
    ];

    let resolved = resolve_preview_candidate(
        "asf.nasa.earthdata::NISAR::granule",
        PreviewProductClass::NisarCompleteRslc,
        &assets,
    )
    .expect("the classified main Preview is authoritative");
    assert_eq!(resolved.source_asset_role, PreviewAssetRole::Preview);
    assert_eq!(resolved.source_asset_key, "product::ASSET::latlon-main");

    let error = resolve_preview_candidate(
        "asf.nasa.earthdata::NISAR::granule",
        PreviewProductClass::NisarCompleteRslc,
        &assets[1..],
    )
    .expect_err("Browse and thumbnail are not NISAR main-preview fallbacks");
    assert_eq!(error.code(), "provider_no_preview");
}

#[test]
fn unsafe_or_ambiguous_preview_evidence_fails_closed() {
    let ambiguous = vec![
        asset(
            AssetRole::Browse,
            "browse-a",
            Some("https://datapool.asf.alaska.edu/a.jpg"),
            "image/jpeg",
        ),
        asset(
            AssetRole::Browse,
            "browse-b",
            Some("https://datapool.asf.alaska.edu/b.jpg"),
            "image/jpeg",
        ),
    ];
    assert_eq!(
        resolve_preview_candidate(
            "asf.catalog::SENTINEL-1::scene",
            PreviewProductClass::Sentinel1Grd,
            &ambiguous,
        )
        .expect_err("ambiguous Browse assets are not guessed")
        .code(),
        "provider_no_preview"
    );

    let derived_only = [asset(
        AssetRole::Derived,
        "derived",
        Some("https://datapool.asf.alaska.edu/derived.jpg"),
        "image/jpeg",
    )];
    assert_eq!(
        resolve_preview_candidate(
            "asf.catalog::SENTINEL-1::scene",
            PreviewProductClass::Sentinel1Grd,
            &derived_only,
        )
        .expect_err("derived products are not genuine Preview assets")
        .code(),
        "provider_no_preview"
    );
}
