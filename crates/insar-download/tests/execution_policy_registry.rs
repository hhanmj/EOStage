use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    SnapshotItem,
};
use insar_download::{
    build_landsat_c2_l2_bundle_access, build_offline_cdse_archive_access,
    build_planetary_computer_landsat_asset_access, classify_frozen_execution,
    AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry, CatalogExecutionPolicyError,
    CatalogExecutionPolicyInput, CatalogExecutionPolicyRegistry, CatalogPolicyContext,
    CdseArchiveEvidence, FrozenExecution, CDSE_SENTINEL2_L2A_POLICY_ID,
    PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID, PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID,
    USGS_LANDSAT_C2_L2_POLICY_ID, USGS_LANDSAT_C2_L2_POLICY_ID_V1, USGS_LANDSAT_C2_L2_RESOLVER_ID,
    USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
};
use insar_provider::{
    catalog::{
        CollectionProfileRegistry, CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID,
        LANDSAT_C2_L2_SCENE_PROFILE_ID, SENTINEL2_L2A_PROFILE_ID,
    },
    cdse::archive_filename::evaluate_archive_filename,
    planetary_computer::{PlanetaryComputerLandsatAsset, PlanetaryComputerLandsatManifest},
    usgs_m2m::{
        UsgsM2mBundleAvailability, UsgsM2mFullBundleOption, LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
        LANDSAT_C2_L2_BUNDLE_OPTION_ID, LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
        LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME, LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM,
        LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE,
    },
};

#[path = "support/landsat_native_policy.rs"]
mod landsat_native_policy;
use serde_json::{json, Map, Value};

fn descriptor() -> AssetAccessDescriptor {
    AssetAccessDescriptor {
        resolver_id: "cdse.archive.resolver.v1".to_string(),
        scheme: AssetAccessScheme::OauthBearerHttps,
        stable_href: Some(
            "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value".to_string(),
        ),
        requires_credentials: true,
        credential_profile: Some("copernicus-dataspace-default".to_string()),
        resolver_parameters: AssetResolverParameters {},
    }
}

fn snapshot(online: bool) -> SnapshotItem {
    let product_id = "00000000-0000-4000-8000-000000000001";
    let product_key = provider_product_key("copernicus.dataspace.odata", "SENTINEL-2", product_id)
        .expect("fixture product key");
    let archive_key = stable_asset_key(&product_key, "archive").expect("fixture archive key");
    SnapshotItem {
        item_id: product_key.clone(),
        provider_id: "copernicus.dataspace.odata".to_owned(),
        display_name: "S2_FIXTURE_ONE.SAFE".to_owned(),
        acquisition_datetime: Some("2026-01-01T10:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String("optical.sentinel-2.l2a".to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String("SENTINEL-2".to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String("cdse.sentinel-2.l2a.collection.v1".to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String("cdse.sentinel-2.l2a.identity.v1".to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String("cdse.sentinel-2.l2a.archive-filename.v1".to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(product_id.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(product_key),
            ),
            (
                "properties".to_owned(),
                json!({
                    "product_type": "S2MSI2A",
                    "online": online,
                    "checksums": [{
                        "algorithm": "MD5",
                        "value": "00000000000000000000000000000000"
                    }]
                }),
            ),
            (
                "assets".to_owned(),
                json!([{
                    "asset_key": "archive",
                    "stable_asset_key": archive_key,
                    "role": "primary",
                    "native_roles": ["archive"],
                    "size_bytes": 2048
                }]),
            ),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn evidence() -> CdseArchiveEvidence {
    let filename = evaluate_archive_filename(
        "S2_FIXTURE_ONE.SAFE",
        Some("attachment; filename=\"S2_FIXTURE_ONE.SAFE.zip\""),
    )
    .expect("fixed Content-Disposition evidence");
    assert_eq!(
        filename.rule_id,
        CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID
    );
    CdseArchiveEvidence {
        filename_evidence: Some(filename),
        expected_size_bytes: Some(2048),
        checksum_algorithm: Some("MD5".to_owned()),
        checksum_value: Some("00000000000000000000000000000000".to_owned()),
    }
}

fn landsat_snapshot() -> SnapshotItem {
    let entity_id = "LC91230452026001LGN00";
    let product_key = provider_product_key("usgs.eros.m2m", "landsat_ot_c2_l2", entity_id)
        .expect("fixture Landsat key");
    SnapshotItem {
        item_id: product_key.clone(),
        provider_id: "usgs.eros.m2m".to_owned(),
        display_name: "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        acquisition_datetime: Some("2026-01-01T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::from_iter([
            (
                "logical_dataset".to_owned(),
                Value::String("optical.landsat.c2.l2".to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String("landsat_ot_c2_l2".to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String("usgs.eros.m2m.landsat.c2.l2.identity.v1".to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String("usgs.eros.m2m.landsat.c2.l2.full-bundle-filename.v1".to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(entity_id.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(product_key),
            ),
            (
                "source_kind".to_owned(),
                Value::String("landsat".to_owned()),
            ),
            (
                "product_kind".to_owned(),
                Value::String("optical_scene".to_owned()),
            ),
            (
                "properties".to_owned(),
                json!({
                    "entity_id": entity_id,
                    "landsat_product_id": "LC09_L2SP_123045_20260101_20260103_02_T1",
                    "dataset_alias": "landsat_ot_c2_l2",
                    "processing_level": "l2sp"
                }),
            ),
        ]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn landsat_option() -> UsgsM2mFullBundleOption {
    UsgsM2mFullBundleOption {
        dataset_alias: "landsat_ot_c2_l2".to_owned(),
        dataset_id: "5e83d14f2fc39685".to_owned(),
        entity_id: "LC91230452026001LGN00".to_owned(),
        display_id: "LC09_L2SP_123045_20260101_20260103_02_T1".to_owned(),
        option_id: LANDSAT_C2_L2_BUNDLE_OPTION_ID.to_owned(),
        product_name: LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME.to_owned(),
        product_code: LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE.to_owned(),
        download_system: LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM.to_owned(),
        availability: UsgsM2mBundleAvailability::Available,
        expected_file_name: "LC09_L2SP_123045_20260101_20260103_02_T1.tar".to_owned(),
        expected_size_bytes: Some(958_605_073),
        checksum_algorithm: Some("sha512".to_owned()),
        checksum_value: Some("a".repeat(128)),
    }
}

fn current_landsat_option() -> UsgsM2mFullBundleOption {
    let mut option = landsat_option();
    option.option_id = "632210d4770592cf".to_owned();
    option.product_code = LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE.to_owned();
    option.download_system = LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM.to_owned();
    option.checksum_algorithm = None;
    option.checksum_value = None;
    option
}

fn executable_landsat_snapshot() -> SnapshotItem {
    let mut item = landsat_snapshot();
    item.asset_access = Some(
        build_landsat_c2_l2_bundle_access(
            &item,
            &landsat_option(),
            CatalogPolicyContext::Production,
        )
        .expect("reviewed Landsat bundle authority"),
    );
    item.execution_policy = Some(AssetExecutionPolicy {
        authority: AssetExecutionAuthority::ResolverAuthoritative,
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID.to_owned(),
        collection_profile_id: Some(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
    });
    item
}

fn planetary_computer_manifest() -> PlanetaryComputerLandsatManifest {
    let display_id = "LC09_L2SP_123045_20260101_20260103_02_T1";
    let definitions = [
        ("coastal", "_SR_B1.TIF", AssetRole::Primary),
        ("blue", "_SR_B2.TIF", AssetRole::Primary),
        ("green", "_SR_B3.TIF", AssetRole::Primary),
        ("red", "_SR_B4.TIF", AssetRole::Primary),
        ("nir08", "_SR_B5.TIF", AssetRole::Primary),
        ("swir16", "_SR_B6.TIF", AssetRole::Primary),
        ("swir22", "_SR_B7.TIF", AssetRole::Primary),
        ("lwir11", "_ST_B10.TIF", AssetRole::Primary),
        ("drad", "_ST_DRAD.TIF", AssetRole::Primary),
        ("emis", "_ST_EMIS.TIF", AssetRole::Primary),
        ("emsd", "_ST_EMSD.TIF", AssetRole::Primary),
        ("trad", "_ST_TRAD.TIF", AssetRole::Primary),
        ("urad", "_ST_URAD.TIF", AssetRole::Primary),
        ("atran", "_ST_ATRAN.TIF", AssetRole::Primary),
        ("cdist", "_ST_CDIST.TIF", AssetRole::Primary),
        ("qa", "_ST_QA.TIF", AssetRole::Auxiliary),
        ("qa_pixel", "_QA_PIXEL.TIF", AssetRole::Auxiliary),
        ("qa_radsat", "_QA_RADSAT.TIF", AssetRole::Auxiliary),
        ("qa_aerosol", "_SR_QA_AEROSOL.TIF", AssetRole::Auxiliary),
        ("ang", "_ANG.txt", AssetRole::Metadata),
        ("mtl.txt", "_MTL.txt", AssetRole::Metadata),
        ("mtl.xml", "_MTL.xml", AssetRole::Metadata),
    ];
    let assets = definitions
        .into_iter()
        .enumerate()
        .map(|(index, (asset_key, suffix, role))| {
            let file_name = format!("{display_id}{suffix}");
            PlanetaryComputerLandsatAsset {
                asset_key: asset_key.to_owned(),
                file_name: file_name.clone(),
                role,
                stable_href: format!(
                    "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/standard/oli-tirs/2026/123/045/{display_id}/{file_name}"
                ),
                expected_size_bytes: 100_000 + index as u64,
            }
        })
        .collect::<Vec<_>>();
    PlanetaryComputerLandsatManifest {
        entity_id: "LC91230452026001LGN00".to_owned(),
        display_id: display_id.to_owned(),
        stac_item_id: "LC09_L2SP_123045_20260101_02_T1".to_owned(),
        processing_level: "l2sp".to_owned(),
        total_size_bytes: assets.iter().map(|asset| asset.expected_size_bytes).sum(),
        assets,
    }
}

fn executable_planetary_computer_asset(index: usize) -> SnapshotItem {
    let source = landsat_snapshot();
    let manifest = planetary_computer_manifest();
    let asset = &manifest.assets[index];
    let access = build_planetary_computer_landsat_asset_access(&source, &manifest, asset)
        .expect("reviewed public Landsat asset authority");
    let mut metadata = source.metadata.clone();
    metadata.insert(
        "source_item_id".to_owned(),
        Value::String(source.item_id.clone()),
    );
    metadata.insert(
        "delivery_asset_key".to_owned(),
        Value::String(asset.asset_key.clone()),
    );
    metadata.insert(
        "delivery_asset_role".to_owned(),
        serde_json::to_value(&asset.role).unwrap(),
    );
    metadata.insert(
        "delivery_source".to_owned(),
        Value::String("planetary_computer".to_owned()),
    );
    SnapshotItem {
        item_id: access.reference.stable_asset_key.clone(),
        provider_id: source.provider_id,
        display_name: asset.file_name.clone(),
        acquisition_datetime: source.acquisition_datetime,
        footprint_geojson: source.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID.to_owned(),
            collection_profile_id: Some(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
        }),
    }
}

#[test]
fn earth_search_profile_cannot_mint_microsoft_download_authority() {
    use insar_provider::catalog::EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID;
    let profile = CollectionProfileRegistry
        .get(EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID)
        .unwrap();
    let mut source = landsat_snapshot();
    let mut manifest = planetary_computer_manifest();
    manifest.entity_id = manifest.stac_item_id.clone();
    source.provider_id = profile.provider_id.to_owned();
    source.item_id = provider_product_key(
        profile.provider_id,
        profile.native_collection_id,
        &manifest.entity_id,
    )
    .unwrap();
    for (key, value) in [
        ("native_collection_id", profile.native_collection_id),
        ("collection_profile_id", profile.profile_id),
        ("identity_rule_id", profile.identity_rule_id),
        ("filename_rule_id", profile.filename_rule_id),
        ("provider_product_id", manifest.entity_id.as_str()),
        ("provider_product_key", source.item_id.as_str()),
    ] {
        source.metadata.insert(key.to_owned(), json!(value));
    }
    source.metadata["properties"]["entity_id"] = json!(manifest.entity_id);
    source.metadata["properties"]["dataset_alias"] = json!(profile.native_collection_id);
    assert_eq!(
        build_planetary_computer_landsat_asset_access(&source, &manifest, &manifest.assets[0])
            .unwrap_err(),
        CatalogExecutionPolicyError::ProviderMismatch
    );
}

#[test]
fn production_cdse_policy_is_valid_for_exact_fixture_and_production_inputs() {
    let registry = CatalogExecutionPolicyRegistry;
    let profiles = CollectionProfileRegistry;
    let profile = profiles.get(SENTINEL2_L2A_PROFILE_ID).unwrap();
    let descriptor = descriptor();
    let input = CatalogExecutionPolicyInput {
        policy_id: CDSE_SENTINEL2_L2A_POLICY_ID,
        collection_profile_id: profile.profile_id,
        normalized_role: AssetRole::Primary,
        descriptor: &descriptor,
    };

    registry
        .validate(&input, CatalogPolicyContext::OfflineFixture)
        .expect("approved fixture policy validates");
    registry
        .validate(&input, CatalogPolicyContext::Production)
        .expect("approved production policy validates");
}

#[test]
fn policy_registry_fails_closed_for_profile_access_and_role_mismatches() {
    let registry = CatalogExecutionPolicyRegistry;
    let profiles = CollectionProfileRegistry;
    let profile = profiles.get(SENTINEL2_L2A_PROFILE_ID).unwrap();
    let descriptor = descriptor();
    let mut input = CatalogExecutionPolicyInput {
        policy_id: CDSE_SENTINEL2_L2A_POLICY_ID,
        collection_profile_id: profile.profile_id,
        normalized_role: AssetRole::Primary,
        descriptor: &descriptor,
    };

    input.normalized_role = AssetRole::Browse;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::NormalizedRoleMismatch
    );

    input.normalized_role = AssetRole::Primary;
    let mut wrong_scheme = descriptor.clone();
    wrong_scheme.scheme = AssetAccessScheme::EarthdataHttps;
    input.descriptor = &wrong_scheme;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::SchemeMismatch
    );

    let mut wrong_profile = descriptor.clone();
    wrong_profile.credential_profile = Some("earthdata-default".to_string());
    input.descriptor = &wrong_profile;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::CredentialProfileMismatch
    );

    let mut missing_href = descriptor.clone();
    missing_href.stable_href = None;
    input.descriptor = &missing_href;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::MissingStableHref
    );

    let mut untrusted_href = descriptor.clone();
    untrusted_href.stable_href = Some("https://example.invalid/file".to_owned());
    input.descriptor = &untrusted_href;
    assert!(matches!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::InvalidDescriptor(_)
    ));
}

#[test]
fn policy_registry_resolves_only_the_canonical_profile_id() {
    let registry = CatalogExecutionPolicyRegistry;
    let profiles = CollectionProfileRegistry;
    let profile = profiles.get(SENTINEL2_L2A_PROFILE_ID).unwrap();
    let descriptor = descriptor();
    let input = CatalogExecutionPolicyInput {
        policy_id: CDSE_SENTINEL2_L2A_POLICY_ID,
        collection_profile_id: "other-profile",
        normalized_role: AssetRole::Primary,
        descriptor: &descriptor,
    };
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::OfflineFixture)
            .unwrap_err(),
        CatalogExecutionPolicyError::ProfileMismatch
    );

    let definition = registry.get(CDSE_SENTINEL2_L2A_POLICY_ID).unwrap();
    assert_eq!(definition.collection_profile_id, profile.profile_id);
    assert_eq!(profile.resolver_policy_id, definition.policy_id);
    assert!(profile.production_enabled);
}

#[test]
fn oauth_bearer_descriptor_requires_a_safe_absolute_https_locator() {
    for stable_href in [
        None,
        Some("relative/path"),
        Some("http://example.invalid/file"),
        Some("ftp://example.invalid/file"),
        Some("file:///tmp/file"),
        Some("https://user@example.invalid/file"),
        Some("https://example.invalid/file?token=not-allowed"),
        Some("https://example.invalid/file#fragment"),
        Some("https://example.invalid:8443/file"),
        Some("https:///missing-host"),
    ] {
        let mut invalid = descriptor();
        invalid.stable_href = stable_href.map(str::to_string);
        assert!(
            invalid.validate().is_err(),
            "{stable_href:?} must be rejected"
        );
    }

    let mut missing_profile = descriptor();
    missing_profile.credential_profile = None;
    assert!(missing_profile.validate().is_err());
}

#[test]
fn offline_cdse_access_builder_requires_online_frozen_archive_evidence() {
    let frozen = build_offline_cdse_archive_access(&snapshot(true), evidence())
        .expect("online CDSE archive evidence builds a frozen access descriptor");
    assert_eq!(
        frozen.descriptor.stable_href.as_deref(),
        Some(
            "https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value"
        )
    );
    assert_eq!(frozen.file_name.as_deref(), Some("S2_FIXTURE_ONE.SAFE.zip"));
    assert_eq!(frozen.expected_size_bytes, Some(2048));
    assert_eq!(frozen.checksum_algorithm.as_deref(), Some("MD5"));

    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(false), evidence()).unwrap_err(),
        CatalogExecutionPolicyError::AssetOffline
    );
    let mut missing_filename = evidence();
    missing_filename.filename_evidence = None;
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), missing_filename).unwrap_err(),
        CatalogExecutionPolicyError::MissingFileName
    );
    let mut missing_size = evidence();
    missing_size.expected_size_bytes = None;
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), missing_size).unwrap_err(),
        CatalogExecutionPolicyError::MissingExpectedSize
    );
    let mut missing_checksum = evidence();
    missing_checksum.checksum_value = None;
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), missing_checksum).unwrap_err(),
        CatalogExecutionPolicyError::MissingChecksum
    );
    let mut mismatched_size = evidence();
    mismatched_size.expected_size_bytes = Some(4096);
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), mismatched_size).unwrap_err(),
        CatalogExecutionPolicyError::FrozenEvidenceMismatch
    );
    let mut unsafe_filename = evidence();
    unsafe_filename
        .filename_evidence
        .as_mut()
        .expect("filename evidence")
        .basename = "../unsafe.SAFE.zip".to_owned();
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), unsafe_filename).unwrap_err(),
        CatalogExecutionPolicyError::UnsafeFileName
    );
    let mut foreign_filename = evidence();
    foreign_filename
        .filename_evidence
        .as_mut()
        .expect("filename evidence")
        .basename = "S2_OTHER.SAFE.zip".to_owned();
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), foreign_filename).unwrap_err(),
        CatalogExecutionPolicyError::FrozenEvidenceMismatch
    );
    let mut foreign_source = evidence();
    foreign_source
        .filename_evidence
        .as_mut()
        .expect("filename evidence")
        .odata_name = "S2_OTHER.SAFE".to_owned();
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), foreign_source).unwrap_err(),
        CatalogExecutionPolicyError::FrozenEvidenceMismatch
    );
    let mut foreign_rule = evidence();
    foreign_rule
        .filename_evidence
        .as_mut()
        .expect("filename evidence")
        .rule_id = "wrong.filename.rule";
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), foreign_rule).unwrap_err(),
        CatalogExecutionPolicyError::FrozenEvidenceMismatch
    );
    let mut mismatched_checksum = evidence();
    mismatched_checksum.checksum_value = Some("ffffffffffffffffffffffffffffffff".to_owned());
    assert_eq!(
        build_offline_cdse_archive_access(&snapshot(true), mismatched_checksum).unwrap_err(),
        CatalogExecutionPolicyError::FrozenEvidenceMismatch
    );
}

#[test]
fn landsat_full_bundle_policy_freezes_only_the_exact_m2m_authority() {
    let access = build_landsat_c2_l2_bundle_access(
        &landsat_snapshot(),
        &landsat_option(),
        CatalogPolicyContext::Production,
    )
    .expect("reviewed Landsat bundle authority");
    assert_eq!(access.reference.asset_key, LANDSAT_C2_L2_BUNDLE_OPTION_ID);
    assert_eq!(
        access.descriptor.resolver_id,
        USGS_LANDSAT_C2_L2_RESOLVER_ID
    );
    assert_eq!(access.descriptor.scheme, AssetAccessScheme::UsgsM2m);
    assert_eq!(
        access.descriptor.credential_profile.as_deref(),
        Some("usgs.eros.m2m.session.v1")
    );
    assert_eq!(
        access.file_name.as_deref(),
        Some("LC09_L2SP_123045_20260101_20260103_02_T1.tar")
    );
    assert_eq!(access.expected_size_bytes, Some(958_605_073));
    let serialized = serde_json::to_string(&access).unwrap().to_ascii_lowercase();
    for forbidden in [
        "https://",
        "requestsignature",
        "downloadid",
        "x-auth-token",
        "authorization",
    ] {
        assert!(!serialized.contains(forbidden), "{forbidden}");
    }

    let mut legacy_scene = landsat_snapshot();
    legacy_scene.metadata.insert(
        "filename_rule_id".to_owned(),
        Value::String("usgs.eros.m2m.landsat.c2.l2.filename-deferred.v1".to_owned()),
    );
    build_landsat_c2_l2_bundle_access(
        &legacy_scene,
        &landsat_option(),
        CatalogPolicyContext::Production,
    )
    .expect("pre-C1 scene batches remain valid source evidence");
}

#[test]
fn historical_landsat_v1_bundle_remains_runtime_admitted_without_widening_v1() {
    let mut item = landsat_snapshot();
    let mut access = build_landsat_c2_l2_bundle_access(
        &item,
        &landsat_option(),
        CatalogPolicyContext::Production,
    )
    .expect("reviewed Landsat bundle authority");
    access.descriptor.resolver_id = USGS_LANDSAT_C2_L2_RESOLVER_ID_V1.to_owned();
    access.descriptor.stable_href = Some(format!(
        "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{}:full_bundle:{}:available:{}",
        landsat_option().entity_id,
        LANDSAT_C2_L2_BUNDLE_OPTION_ID,
        landsat_option().display_id,
    ));
    item.asset_access = Some(access);
    item.execution_policy = Some(AssetExecutionPolicy {
        authority: AssetExecutionAuthority::ResolverAuthoritative,
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V1.to_owned(),
        collection_profile_id: Some(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
    });

    assert!(matches!(
        classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        FrozenExecution::ResolverAuthoritative(_)
    ));

    let mut widened = item.clone();
    widened
        .asset_access
        .as_mut()
        .unwrap()
        .descriptor
        .stable_href = Some(format!(
            "urn:usgs:m2m:landsat:landsat_ot_c2_l2:{}:full_bundle:632210d4770592cf:available:{}:D806:dds_ms",
            landsat_option().entity_id,
            landsat_option().display_id,
        ));
    assert!(!matches!(
        classify_frozen_execution(
            &widened,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        FrozenExecution::ResolverAuthoritative(_)
    ));
}

#[test]
fn current_landsat_bundle_variant_freezes_semantics_and_is_runtime_admitted() {
    let mut item = landsat_snapshot();
    let access = build_landsat_c2_l2_bundle_access(
        &item,
        &current_landsat_option(),
        CatalogPolicyContext::Production,
    )
    .expect("current reviewed Landsat bundle authority");
    assert_eq!(access.reference.asset_key, "632210d4770592cf");
    let locator = access
        .descriptor
        .stable_href
        .clone()
        .expect("frozen M2M locator");
    assert!(locator.ends_with(":D806:dds_ms"));

    item.asset_access = Some(access);
    item.execution_policy = Some(AssetExecutionPolicy {
        authority: AssetExecutionAuthority::ResolverAuthoritative,
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID.to_owned(),
        collection_profile_id: Some(LANDSAT_C2_L2_SCENE_PROFILE_ID.to_owned()),
    });
    assert!(matches!(
        classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        FrozenExecution::ResolverAuthoritative(_)
    ));

    let mut forged = item.clone();
    forged.asset_access.as_mut().unwrap().descriptor.stable_href =
        Some(locator.replace(":D806:dds_ms", ":D806:folder"));
    assert!(!matches!(
        classify_frozen_execution(
            &forged,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        FrozenExecution::ResolverAuthoritative(_)
    ));
}

#[test]
fn landsat_full_bundle_is_admitted_by_the_download_runtime_policy() {
    let item = executable_landsat_snapshot();

    assert!(matches!(
        classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        FrozenExecution::ResolverAuthoritative(_)
    ));
}

#[test]
fn planetary_computer_landsat_assets_are_anonymous_closed_and_runtime_admitted() {
    let manifest = planetary_computer_manifest();
    assert_eq!(manifest.assets.len(), 22);
    for (index, asset) in manifest.assets.iter().enumerate() {
        let item = executable_planetary_computer_asset(index);
        let access = item.asset_access.as_ref().unwrap();
        assert_eq!(access.reference.asset_key, asset.asset_key);
        assert_eq!(
            access.descriptor.resolver_id,
            PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID
        );
        assert_eq!(access.descriptor.scheme, AssetAccessScheme::PublicHttps);
        assert!(!access.descriptor.requires_credentials);
        assert!(access.descriptor.credential_profile.is_none());
        assert_eq!(access.expected_size_bytes, Some(asset.expected_size_bytes));
        assert!(access.checksum_algorithm.is_none());
        assert!(access.checksum_value.is_none());
        assert!(!access
            .descriptor
            .stable_href
            .as_deref()
            .unwrap()
            .contains('?'));
        assert!(matches!(
            classify_frozen_execution(
                &item,
                &AsfCollectionProfileRegistry,
                &AssetExecutionPolicyRegistry,
            ),
            FrozenExecution::ResolverAuthoritative(_)
        ));
    }
}

#[test]
fn planetary_computer_landsat_asset_drift_never_falls_back_to_legacy() {
    let mutations: &[fn(&mut SnapshotItem)] = &[
        |item| item.item_id.push_str("-other"),
        |item| {
            item.asset_access.as_mut().unwrap().role = AssetRole::Metadata;
        },
        |item| {
            item.asset_access
                .as_mut()
                .unwrap()
                .descriptor
                .stable_href
                .as_mut()
                .unwrap()
                .push_str("?sig=persisted-secret");
        },
        |item| {
            item.asset_access
                .as_mut()
                .unwrap()
                .descriptor
                .credential_profile = Some("unexpected".to_owned());
            item.asset_access
                .as_mut()
                .unwrap()
                .descriptor
                .requires_credentials = true;
        },
    ];
    for mutate in mutations {
        let mut item = executable_planetary_computer_asset(1);
        mutate(&mut item);
        assert!(matches!(
            classify_frozen_execution(
                &item,
                &AsfCollectionProfileRegistry,
                &AssetExecutionPolicyRegistry,
            ),
            FrozenExecution::Invalid(_)
        ));
    }
}

#[test]
fn landsat_runtime_policy_rejects_every_mutated_authority_binding_without_legacy_fallback() {
    type Mutation = fn(&mut SnapshotItem);
    let cases: &[(&str, Mutation)] = &[
        ("provider", |item| {
            item.provider_id = "other.provider".to_owned()
        }),
        ("dataset", |item| {
            item.metadata.insert(
                "logical_dataset".to_owned(),
                Value::String("optical.other".to_owned()),
            );
        }),
        ("native_collection", |item| {
            item.metadata.insert(
                "native_collection_id".to_owned(),
                Value::String("other_collection".to_owned()),
            );
        }),
        ("metadata_profile", |item| {
            item.metadata.insert(
                "collection_profile_id".to_owned(),
                Value::String("other.profile".to_owned()),
            );
        }),
        ("policy_profile", |item| {
            item.execution_policy
                .as_mut()
                .expect("execution policy")
                .collection_profile_id = Some("other.profile".to_owned());
        }),
        ("identity_rule", |item| {
            item.metadata.insert(
                "identity_rule_id".to_owned(),
                Value::String("other.identity".to_owned()),
            );
        }),
        ("delivery", |item| {
            item.metadata.insert(
                "delivery_model".to_owned(),
                Value::String("other_delivery".to_owned()),
            );
        }),
        ("processing_level", |item| {
            item.metadata
                .get_mut("properties")
                .and_then(Value::as_object_mut)
                .expect("Landsat properties")
                .insert(
                    "processing_level".to_owned(),
                    Value::String("l1tp".to_owned()),
                );
        }),
        ("role", |item| {
            item.asset_access.as_mut().expect("asset access").role = AssetRole::Browse;
        }),
        ("scheme", |item| {
            item.asset_access
                .as_mut()
                .expect("asset access")
                .descriptor
                .scheme = AssetAccessScheme::EarthdataHttps;
        }),
        ("resolver", |item| {
            item.asset_access
                .as_mut()
                .expect("asset access")
                .descriptor
                .resolver_id = "earthdata_https".to_owned();
        }),
        ("credential", |item| {
            item.asset_access
                .as_mut()
                .expect("asset access")
                .descriptor
                .credential_profile = Some("earthdata-default".to_owned());
        }),
        ("locator", |item| {
            item.asset_access
                .as_mut()
                .expect("asset access")
                .descriptor
                .stable_href = Some("https://example.invalid/arbitrary.tar".to_owned());
        }),
    ];

    for (name, mutate) in cases {
        let mut item = executable_landsat_snapshot();
        mutate(&mut item);
        let classified = classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        );
        assert!(matches!(classified, FrozenExecution::Invalid(_)), "{name}");
    }

    let mut item = executable_landsat_snapshot();
    item.asset_access
        .as_mut()
        .expect("asset access")
        .descriptor
        .stable_href = Some("https://example.invalid/arbitrary.tar".to_owned());
    let outcome = insar_download::route_frozen_execution(
        classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        (),
        |_| "legacy",
        |_, _| "resolver",
        |_, _| "invalid",
    );
    assert_eq!(outcome, "invalid");
}

#[test]
fn landsat_policy_rejects_wrong_scheme_resolver_profile_role_and_locator() {
    let profile = CollectionProfileRegistry
        .get(LANDSAT_C2_L2_SCENE_PROFILE_ID)
        .unwrap();
    let frozen = build_landsat_c2_l2_bundle_access(
        &landsat_snapshot(),
        &landsat_option(),
        CatalogPolicyContext::Production,
    )
    .unwrap();
    let registry = CatalogExecutionPolicyRegistry;

    let mut wrong_scheme = frozen.descriptor.clone();
    wrong_scheme.scheme = AssetAccessScheme::EarthdataHttps;
    let mut input = CatalogExecutionPolicyInput {
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID,
        collection_profile_id: profile.profile_id,
        normalized_role: AssetRole::Primary,
        descriptor: &wrong_scheme,
    };
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .unwrap_err(),
        CatalogExecutionPolicyError::SchemeMismatch
    );

    let mut wrong_resolver = frozen.descriptor.clone();
    wrong_resolver.resolver_id = "earthdata_https".to_owned();
    input.descriptor = &wrong_resolver;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .unwrap_err(),
        CatalogExecutionPolicyError::ResolverMismatch
    );

    let mut wrong_profile = frozen.descriptor.clone();
    wrong_profile.credential_profile = Some("earthdata-default".to_owned());
    input.descriptor = &wrong_profile;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .unwrap_err(),
        CatalogExecutionPolicyError::CredentialProfileMismatch
    );

    let mut wrong_locator = frozen.descriptor.clone();
    wrong_locator.stable_href = Some(
        "urn:usgs:m2m:landsat:landsat_ot_c2_l2:LC91230452026001LGN00:selected_band:5e83d14fec7cae84:available"
            .to_owned(),
    );
    input.descriptor = &wrong_locator;
    assert!(matches!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .unwrap_err(),
        CatalogExecutionPolicyError::InvalidDescriptor(_)
    ));
    let mut foreign_option = landsat_option();
    foreign_option.entity_id = "LC80000000000000000000".to_owned();
    assert_eq!(
        build_landsat_c2_l2_bundle_access(
            &landsat_snapshot(),
            &foreign_option,
            CatalogPolicyContext::Production,
        )
        .unwrap_err(),
        CatalogExecutionPolicyError::ArchiveIdentityMismatch
    );

    input.descriptor = &frozen.descriptor;
    input.normalized_role = AssetRole::Browse;
    assert_eq!(
        registry
            .validate(&input, CatalogPolicyContext::Production)
            .unwrap_err(),
        CatalogExecutionPolicyError::NormalizedRoleMismatch
    );
}

#[test]
fn sentinel3_and_sentinel2_l1c_frozen_snapshots_pass_cutover_eligibility() {
    use insar_download::{
        build_cdse_archive_access, evaluate_resolver_cutover_eligibility,
        ResolverCutoverEligibility,
    };
    use insar_provider::{
        catalog::{
            CDSE_SENTINEL2_L1C_ARCHIVE_FILENAME_RULE_ID, CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID,
            CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID,
            CDSE_SENTINEL3_STANDARD_ARCHIVE_FILENAME_RULE_ID,
            CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID, CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID,
            SENTINEL2_L1C_PROFILE_ID, SENTINEL3_STANDARD_PROFILE_ID,
        },
        cdse::archive_filename::archive_filename_from_odata_name_for_rule,
    };

    let product_id = "00000000-0000-4000-8000-000000000009";
    let cases = [
        (
            "optical.sentinel-3.standard",
            "SENTINEL-3",
            SENTINEL3_STANDARD_PROFILE_ID,
            CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID,
            CDSE_SENTINEL3_STANDARD_ARCHIVE_FILENAME_RULE_ID,
            CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID,
            "OL_1_EFR___",
            "S3A_OL_1_EFR____20261124T020000_20261124T020300.SEN3",
        ),
        (
            "optical.sentinel-2.l1c",
            "SENTINEL-2",
            SENTINEL2_L1C_PROFILE_ID,
            CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID,
            CDSE_SENTINEL2_L1C_ARCHIVE_FILENAME_RULE_ID,
            CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID,
            "S2MSI1C",
            "S2A_MSIL1C_20261124T020000_N0511_R017_T49QDB_20261124T040000.SAFE",
        ),
    ];

    for (
        logical_dataset,
        native_collection_id,
        profile_id,
        identity_rule_id,
        filename_rule_id,
        policy_id,
        product_type,
        display_name,
    ) in cases
    {
        let product_key = provider_product_key(
            "copernicus.dataspace.odata",
            native_collection_id,
            product_id,
        )
        .expect("fixture product key");
        let archive_key = stable_asset_key(&product_key, "archive").expect("fixture archive key");
        let checksum_value = "0123456789abcdef0123456789abcdef";
        let mut item = SnapshotItem {
            item_id: product_key.clone(),
            provider_id: "copernicus.dataspace.odata".to_owned(),
            display_name: display_name.to_owned(),
            acquisition_datetime: Some("2026-11-24T02:00:00Z".to_owned()),
            footprint_geojson: None,
            metadata: Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    Value::String(logical_dataset.to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(native_collection_id.to_owned()),
                ),
                (
                    "collection_profile_id".to_owned(),
                    Value::String(profile_id.to_owned()),
                ),
                (
                    "identity_rule_id".to_owned(),
                    Value::String(identity_rule_id.to_owned()),
                ),
                (
                    "filename_rule_id".to_owned(),
                    Value::String(filename_rule_id.to_owned()),
                ),
                (
                    "delivery_model".to_owned(),
                    Value::String("archive_first".to_owned()),
                ),
                (
                    "provider_product_id".to_owned(),
                    Value::String(product_id.to_owned()),
                ),
                (
                    "provider_product_key".to_owned(),
                    Value::String(product_key),
                ),
                (
                    "properties".to_owned(),
                    json!({
                        "product_type": product_type,
                        "online": true,
                        "checksums": [{ "algorithm": "MD5", "value": checksum_value }],
                    }),
                ),
                (
                    "assets".to_owned(),
                    json!([{
                        "asset_key": "archive",
                        "stable_asset_key": archive_key,
                        "role": "primary",
                        "native_roles": ["archive"],
                        "size_bytes": 123456,
                    }]),
                ),
            ]),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        };
        let filename_evidence =
            archive_filename_from_odata_name_for_rule(display_name, filename_rule_id)
                .expect("archive filename evidence");
        let access = build_cdse_archive_access(
            &item,
            CdseArchiveEvidence {
                filename_evidence: Some(filename_evidence),
                expected_size_bytes: Some(123456),
                checksum_algorithm: Some("MD5".to_owned()),
                checksum_value: Some(checksum_value.to_owned()),
            },
            CatalogPolicyContext::Production,
        )
        .expect("build_cdse_archive_access should succeed");
        item.asset_access = Some(access);
        item.execution_policy = Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: policy_id.to_owned(),
            collection_profile_id: Some(profile_id.to_owned()),
        });

        let eligibility = evaluate_resolver_cutover_eligibility(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        );
        assert!(
            matches!(eligibility, ResolverCutoverEligibility::Eligible(_)),
            "expected Eligible for {logical_dataset}, got {eligibility:?}"
        );
        let frozen = classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        );
        assert!(
            matches!(frozen, FrozenExecution::ResolverAuthoritative(_)),
            "expected FrozenExecution::ResolverAuthoritative for {logical_dataset}, got {frozen:?}"
        );
    }
}
