use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use insar_core::{
    stable_asset_key, AssetExecutionAuthority, AssetExecutionPolicy, AssetRole, DownloadOutcome,
    FrozenTaskSnapshot, SnapshotItem,
};
use insar_download::{
    build_burst_frozen_asset_access, evaluate_resolver_cutover_eligibility,
    AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry, AssetResolverRegistry,
    AuthRefreshPolicyId, CredentialMaterial, CredentialProvider, CredentialProviderError,
    DownloadWork, DownloadWorker, ExecutionDeliveryModel, ProductionDownloadWorker, ResolvedAsset,
    ResolvedTransferResult, ResolvedTransferSession, ResolverCutoverEligibility,
    ResolverExecutionRuntime, SystemClock, TransferAuth, TransferBackend, TransferBackendKind,
    TransferOptions, TransferRequest, TransferResult, TransferUpdate, WorkerControl,
    ASF_BURST_ASSET_EXECUTION_POLICY_ID, ASF_BURST_COLLECTION_PROFILE_ID,
    EARTHDATA_BURST_CREDENTIAL_PROFILE,
};
use insar_provider::asf::collection_profile::{profile_for_id, AsfDeliveryModel, AsfItemModel};
use serde_json::{json, Map, Value};

const SOURCE_ITEM_ID: &str = "ASF::SLC-BURST::S1_000001_IW2_20260820T000000_VV_A1B2-BURST";
const PROVIDER_PRODUCT_ID: &str = "G12345-ASF";
const NATIVE_PRODUCT_ID: &str = "S1_000001_IW2_20260820T000000_VV_A1B2-BURST";
const TIFF_NAME: &str = "measurement.tiff";
const XML_NAME: &str = "metadata.xml";

fn burst_item(role: AssetRole) -> SnapshotItem {
    let (role_name, file_name, media_type, size_bytes) = match role {
        AssetRole::Primary => ("primary", TIFF_NAME, "image/tiff", Some(1_024_u64)),
        AssetRole::Metadata => ("metadata", XML_NAME, "application/xml", None),
        _ => unreachable!(),
    };
    let asset_key = format!("{NATIVE_PRODUCT_ID}:{file_name}");
    let stable_key = stable_asset_key(SOURCE_ITEM_ID, &asset_key).unwrap();
    let primary_stable_key =
        stable_asset_key(SOURCE_ITEM_ID, &format!("{NATIVE_PRODUCT_ID}:{TIFF_NAME}")).unwrap();
    let metadata_stable_key =
        stable_asset_key(SOURCE_ITEM_ID, &format!("{NATIVE_PRODUCT_ID}:{XML_NAME}")).unwrap();
    let href = format!("https://sentinel1-burst.asf.alaska.edu/{NATIVE_PRODUCT_ID}/{file_name}");
    let mut metadata = Map::from_iter([
        ("provider".to_owned(), json!("asf.catalog")),
        ("dataset".to_owned(), json!("SLC-BURST")),
        ("product_family".to_owned(), json!("BURST")),
        ("processing_level".to_owned(), json!("L1")),
        (
            "identity_rule_id".to_owned(),
            json!(ASF_BURST_COLLECTION_PROFILE_ID),
        ),
        ("product_key".to_owned(), json!(SOURCE_ITEM_ID)),
        ("provider_product_key".to_owned(), json!(SOURCE_ITEM_ID)),
        ("provider_product_id".to_owned(), json!(PROVIDER_PRODUCT_ID)),
        ("product_id".to_owned(), json!(NATIVE_PRODUCT_ID)),
        ("scene_id".to_owned(), json!(NATIVE_PRODUCT_ID)),
        ("source_item_id".to_owned(), json!(SOURCE_ITEM_ID)),
        ("stable_asset_key".to_owned(), json!(stable_key)),
        (
            "stable_asset_keys".to_owned(),
            json!([primary_stable_key, metadata_stable_key]),
        ),
        ("platform".to_owned(), json!("SENTINEL-1A")),
        ("instrument".to_owned(), json!("C-SAR")),
        ("beam_mode".to_owned(), json!("IW")),
        ("beam_swath".to_owned(), json!("IW2")),
        ("polarization".to_owned(), json!("VV")),
        ("flight_direction".to_owned(), json!("ASCENDING")),
        ("look_direction".to_owned(), json!("RIGHT")),
        ("relative_orbit".to_owned(), json!(141)),
        ("absolute_orbit".to_owned(), json!(57_142)),
        ("path".to_owned(), json!(141)),
        (
            "burst".to_owned(),
            json!({
                "burst_id": null,
                "full_burst_id": "141_301495_IW2",
                "relative_burst_id": 301_495,
                "absolute_burst_id": 5_714_273,
                "subswath": "IW2",
                "burst_index": 0
            }),
        ),
        (
            "native_metadata".to_owned(),
            json!({
                "cmr_granule_concept_id": PROVIDER_PRODUCT_ID,
                "cmr_native_id": NATIVE_PRODUCT_ID,
                "raw_authority": {},
                "collection": {
                    "concept_id": "C2709161906-ASF",
                    "short_name": "SENTINEL-1_BURSTS",
                    "version": "1",
                    "provider_id": "ASF"
                },
                "burst": {
                    "full_id": "141_301495_IW2",
                    "relative_id": 301_495,
                    "absolute_id": 5_714_273,
                    "index": 0,
                    "subswath": "IW2",
                    "byte_length": 1_024
                },
                "scope": {
                    "beam_mode": "IW",
                    "processing_level": "L1",
                    "processing_type": "BURST",
                    "path": 141,
                    "flight_direction": "ASCENDING",
                    "platform": "SENTINEL-1A",
                    "instrument": "C-SAR",
                    "look_direction": "RIGHT"
                },
                "browse": null,
                "preview_available": false
            }),
        ),
        ("asset_role".to_owned(), json!(role_name)),
        ("official_href".to_owned(), json!(href)),
        ("asset_basename".to_owned(), json!(file_name)),
        ("media_type".to_owned(), json!(media_type)),
        (
            "size_bytes".to_owned(),
            size_bytes.map(Value::from).unwrap_or(Value::Null),
        ),
        ("burst_asset_policy".to_owned(), json!("all")),
    ]);
    let mut payload = Map::from_iter([
        ("burst_asset".to_owned(), json!(true)),
        ("download_url".to_owned(), json!(href)),
        ("file_name".to_owned(), json!(file_name)),
        ("media_type".to_owned(), json!(media_type)),
        ("source_item_id".to_owned(), json!(SOURCE_ITEM_ID)),
        ("asset_role".to_owned(), json!(role_name)),
        ("burst_asset_policy".to_owned(), json!("all")),
    ]);
    if let Some(size_bytes) = size_bytes {
        payload.insert("file_size_remote".to_owned(), Value::from(size_bytes));
    }
    SnapshotItem {
        item_id: metadata["stable_asset_key"].as_str().unwrap().to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: file_name.to_owned(),
        acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: std::mem::take(&mut metadata),
        payload,
        asset_access: None,
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: ASF_BURST_ASSET_EXECUTION_POLICY_ID.to_owned(),
            collection_profile_id: Some(ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
        }),
    }
}

#[test]
fn task_3c1_tiff_and_xml_evidence_build_exact_frozen_asset_access() {
    for role in [AssetRole::Primary, AssetRole::Metadata] {
        let item = burst_item(role.clone());
        let access = build_burst_frozen_asset_access(&item).unwrap();
        let file_name = match role {
            AssetRole::Primary => TIFF_NAME,
            AssetRole::Metadata => XML_NAME,
            _ => unreachable!(),
        };
        assert_eq!(access.reference.provider_id, "asf.catalog");
        assert_eq!(access.reference.collection_id, "SLC-BURST");
        assert_eq!(access.reference.provider_product_id, PROVIDER_PRODUCT_ID);
        assert_eq!(access.reference.provider_product_key, SOURCE_ITEM_ID);
        assert_eq!(
            access.reference.asset_key,
            format!("{NATIVE_PRODUCT_ID}:{file_name}")
        );
        assert_eq!(access.reference.stable_asset_key, item.item_id);
        assert_eq!(access.role, role);
        assert_eq!(access.file_name.as_deref(), Some(file_name));
        assert_eq!(
            access.expected_size_bytes,
            (role == AssetRole::Primary).then_some(1_024)
        );
        assert_eq!(access.checksum_algorithm, None);
        assert_eq!(access.checksum_value, None);
        assert_eq!(access.descriptor.resolver_id, "earthdata_https");
        assert_eq!(
            access.descriptor.credential_profile.as_deref(),
            Some(EARTHDATA_BURST_CREDENTIAL_PROFILE)
        );
        assert!(access.descriptor.requires_credentials);
    }
}

#[test]
fn task_3c1_burst_evidence_mutations_fail_closed_role_by_role() {
    for role in [AssetRole::Primary, AssetRole::Metadata] {
        for label in [
            "provider",
            "profile",
            "identity",
            "missing-role",
            "media",
            "extension",
            "stable-key",
            "missing-href",
            "missing-filename",
            "host",
            "scheme",
            "query",
            "fragment",
            "cross-product",
            "payload-role",
            "payload-href",
            "prebuilt-access",
        ] {
            let mut item = burst_item(role.clone());
            match label {
                "provider" => item.provider_id = "other.provider".to_owned(),
                "profile" => {
                    item.execution_policy
                        .as_mut()
                        .unwrap()
                        .collection_profile_id = Some("asf.sentinel-1.standard.v1".to_owned())
                }
                "identity" => item.metadata["identity_rule_id"] = json!("wrong.identity.v1"),
                "missing-role" => {
                    item.metadata.remove("asset_role");
                }
                "media" => item.metadata["media_type"] = json!("application/octet-stream"),
                "extension" => item.metadata["asset_basename"] = json!("wrong.bin"),
                "stable-key" => item.metadata["stable_asset_key"] = json!("wrong-key"),
                "missing-href" => {
                    item.metadata.remove("official_href");
                }
                "missing-filename" => {
                    item.metadata.remove("asset_basename");
                }
                "host" => {
                    item.metadata["official_href"] = json!(format!(
                        "https://example.invalid/{NATIVE_PRODUCT_ID}/{}",
                        item.display_name
                    ))
                }
                "scheme" => {
                    let href = item.metadata["official_href"].as_str().unwrap();
                    item.metadata["official_href"] = json!(href.replacen("https://", "http://", 1));
                }
                "query" => {
                    let href = item.metadata["official_href"].as_str().unwrap();
                    item.metadata["official_href"] = json!(format!("{href}?token=x"));
                }
                "fragment" => {
                    let href = item.metadata["official_href"].as_str().unwrap();
                    item.metadata["official_href"] = json!(format!("{href}#fragment"));
                }
                "cross-product" => item.metadata["source_item_id"] = json!("other-product"),
                "payload-role" => item.payload["asset_role"] = json!("other"),
                "payload-href" => item.payload["download_url"] = json!("https://example.invalid/x"),
                "prebuilt-access" => {
                    item.asset_access = Some(build_burst_frozen_asset_access(&item).unwrap())
                }
                _ => unreachable!(),
            }
            assert!(
                build_burst_frozen_asset_access(&item).is_err(),
                "{label} {role:?} must fail closed"
            );
        }
    }
}

#[test]
fn coherent_cross_product_parent_rewrite_still_fails_closed() {
    let mut item = burst_item(AssetRole::Primary);
    let forged_source_item_id = "ASF::SLC-BURST::OTHER_NATIVE_BURST";
    for key in ["source_item_id", "product_key", "provider_product_key"] {
        item.metadata[key] = json!(forged_source_item_id);
    }
    item.payload["source_item_id"] = json!(forged_source_item_id);
    let forged_primary_stable_key = stable_asset_key(
        forged_source_item_id,
        &format!("{NATIVE_PRODUCT_ID}:{TIFF_NAME}"),
    )
    .unwrap();
    let forged_metadata_stable_key = stable_asset_key(
        forged_source_item_id,
        &format!("{NATIVE_PRODUCT_ID}:{XML_NAME}"),
    )
    .unwrap();
    item.metadata["stable_asset_keys"] = json!([
        forged_primary_stable_key.clone(),
        forged_metadata_stable_key
    ]);
    item.item_id = forged_primary_stable_key.clone();
    item.metadata["stable_asset_key"] = json!(forged_primary_stable_key);

    assert!(
        build_burst_frozen_asset_access(&item).is_err(),
        "mirrored parent IDs and recomputed child stable key must not replace frozen product authority"
    );
}

#[test]
fn dedicated_burst_policy_and_collection_profile_approve_only_earthdata_https() {
    let profile = profile_for_id(ASF_BURST_COLLECTION_PROFILE_ID).unwrap();
    assert_eq!(profile.item_model, AsfItemModel::MultiAssetProduct);
    assert_eq!(profile.delivery_model, AsfDeliveryModel::MultiAssetProduct);
    assert_eq!(profile.approved_resolver_ids, ["earthdata_https"]);

    let policy = AssetExecutionPolicyRegistry
        .get(ASF_BURST_ASSET_EXECUTION_POLICY_ID)
        .expect("dedicated Burst policy is registered");
    assert_eq!(
        policy.collection_profile_id,
        ASF_BURST_COLLECTION_PROFILE_ID
    );
    assert_eq!(
        policy.delivery_model,
        ExecutionDeliveryModel::Asf(AsfDeliveryModel::MultiAssetProduct)
    );
    assert_eq!(policy.approved_product_families, ["BURST"]);
    assert_eq!(policy.approved_processing_levels, ["L1"]);
    assert_eq!(policy.approved_resolver_ids, ["earthdata_https"]);
    assert_eq!(
        policy.approved_credential_profiles,
        [EARTHDATA_BURST_CREDENTIAL_PROFILE]
    );
    assert_eq!(
        policy.approved_asset_roles,
        [AssetRole::Primary, AssetRole::Metadata]
    );
    assert_eq!(
        policy.auth_refresh_policy_id,
        AuthRefreshPolicyId::EarthdataBurstSessionV1
    );

    for role in [AssetRole::Primary, AssetRole::Metadata] {
        let item = burst_item(role);
        let ResolverCutoverEligibility::Eligible(eligible) = evaluate_resolver_cutover_eligibility(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ) else {
            panic!("exact Burst role must be resolver-authoritative");
        };
        assert_eq!(
            eligible.policy.policy_id,
            ASF_BURST_ASSET_EXECUTION_POLICY_ID
        );
        assert_eq!(
            eligible.frozen_access,
            build_burst_frozen_asset_access(&item).unwrap()
        );
    }
}

struct BurstBasicCredentials;

impl CredentialProvider for BurstBasicCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, EARTHDATA_BURST_CREDENTIAL_PROFILE);
        CredentialMaterial::basic("fixture-user", "fixture-password")
    }
}

#[derive(Default)]
struct RecordingResolverBackend {
    resolved_calls: AtomicUsize,
}

impl TransferBackend for RecordingResolverBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        _request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        panic!("resolver-authoritative Burst item must never enter legacy transfer")
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(1).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.resolved_calls.fetch_add(1, Ordering::SeqCst);
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: 1_024,
            attempt_count: 1,
            failure: None,
        })
    }
}

#[test]
fn production_worker_dispatches_burst_only_through_resolver_authoritative_backend() {
    let item = burst_item(AssetRole::Primary);
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": "burst-step-3-worker-gate",
        "batch_id": "immutable-source-batch",
        "provider_id": "asf.catalog",
        "created_at": "2026-08-26T00:00:00Z",
        "aoi_name": "fixture",
        "aoi_geojson": null,
        "query": {"dataset": "SLC-BURST"},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": std::env::temp_dir().join("burst-step-3-worker-gate"),
        "layout": "direct"
    }))
    .unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies(
        AssetResolverRegistry::with_standard_resolvers().unwrap(),
        Arc::new(BurstBasicCredentials),
        Arc::new(SystemClock),
    );
    let backend = Arc::new(RecordingResolverBackend::default());
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, backend.clone());

    let result = worker.execute(
        DownloadWork { task, item },
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert!(result.failure.is_none());
    assert_eq!(backend.resolved_calls.load(Ordering::SeqCst), 1);
}

/// Bugfix 2026-10-20 (Sentinel-1D bursts): ASF's Sentinel-1D burst records
/// carry no instrument LookDirection, so the parent authority records none.
/// An item that also has none keeps its authority and builds its frozen
/// access. A look direction on one side only is still a mismatch.
#[test]
fn a_burst_without_a_recorded_look_direction_keeps_its_authority() {
    for role in [AssetRole::Primary, AssetRole::Metadata] {
        let mut item = burst_item(role.clone());
        item.metadata.remove("look_direction");
        item.metadata
            .insert("platform".to_owned(), json!("SENTINEL-1D"));
        let scope = item
            .metadata
            .get_mut("native_metadata")
            .and_then(|value| value.get_mut("scope"))
            .and_then(Value::as_object_mut)
            .unwrap();
        scope.insert("look_direction".to_owned(), Value::Null);
        scope.insert("platform".to_owned(), json!("SENTINEL-1D"));
        assert!(
            build_burst_frozen_asset_access(&item).is_ok(),
            "{role:?} without a look direction"
        );
        let mut one_sided = item.clone();
        one_sided
            .metadata
            .insert("look_direction".to_owned(), json!("RIGHT"));
        assert!(build_burst_frozen_asset_access(&one_sided).is_err());
    }
}
