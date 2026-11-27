//! Issue #3 R2 slice B5: freeze search-only HLS v2.0 scenes. Only the
//! reviewed scene identity, acquisition time, footprint, platform and cloud
//! cover cross into the frozen batch; no asset URL, signing or transfer
//! authority exists for these candidates.
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    CapabilityKind, FrozenAssetAccess, ResourceDescriptor, ResourceRegistry, StableAssetReference,
    StaticCapabilityLifecycle,
};
use insar_provider::catalog::{
    mpc_hls_scene_time_matches, valid_mpc_hls_scene_id, CollectionProfileRegistry, MpcHlsProduct,
    MPC_HLS_PROVIDER_ID,
};
use reqwest::Url;

fn hls_error(reason: &str) -> ApplicationServiceError {
    catalogue_provider_error("CATALOGUE_PRODUCT_INVALID", "HLS 场景身份无效", reason)
}

pub(super) fn hls_product_for_query(query: &CatalogSearchQuery) -> Option<MpcHlsProduct> {
    MpcHlsProduct::for_source(
        &query.logical_dataset,
        &query.provider_id,
        &query.native_collection_id,
    )
}

pub(super) fn map_hls_scene_product(
    product: ProviderProduct,
    query: &CatalogSearchQuery,
) -> Result<SnapshotItem, ApplicationServiceError> {
    let hls = hls_product_for_query(query).ok_or_else(|| hls_error("query_source"))?;
    let profile = CollectionProfileRegistry
        .for_source(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
        )
        .ok_or_else(|| hls_error("collection_profile"))?;
    let expected_source_kind = match hls {
        MpcHlsProduct::L30 => DataSourceKind::Landsat,
        MpcHlsProduct::S30 => DataSourceKind::Sentinel2,
    };
    if product.provider_id != MPC_HLS_PROVIDER_ID
        || product.source_kind != expected_source_kind
        || product.product_kind != ProductKind::OpticalScene
        || product.product_id != product.display_name
        || !valid_mpc_hls_scene_id(hls, &product.product_id)
        || !product.payload.is_empty()
        || product.footprint_bbox.is_some()
    {
        return Err(hls_error("product_scope"));
    }
    let platform = product
        .properties
        .get("platform")
        .and_then(Value::as_str)
        .filter(|platform| {
            hls.platforms()
                .iter()
                .any(|(neutral, _)| neutral == platform)
        })
        .ok_or_else(|| hls_error("platform"))?
        .to_owned();
    let cloud_cover = product
        .properties
        .get("cloud_cover")
        .and_then(Value::as_f64)
        .filter(|cloud| cloud.is_finite() && (0.0..=100.0).contains(cloud))
        .ok_or_else(|| hls_error("cloud_cover"))?;
    if product.properties.len() != 2 {
        return Err(hls_error("properties"));
    }
    let acquisition_datetime = product
        .acquisition_datetime
        .clone()
        .filter(|time| mpc_hls_scene_time_matches(&product.product_id, time))
        .ok_or_else(|| hls_error("acquisition_datetime"))?;
    let footprint = product
        .footprint_geojson
        .clone()
        .ok_or_else(|| hls_error("footprint"))?;
    let item_id = provider_product_key(
        &product.provider_id,
        profile.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| hls_error("provider_product_key"))?;
    let mut frozen_assets = Vec::new();
    for asset in &product.assets {
        let Some(asset_key) = asset.asset_key.as_deref() else {
            continue;
        };
        let mut frozen = Map::from_iter([
            ("asset_key".to_owned(), Value::String(asset_key.to_owned())),
            (
                "role".to_owned(),
                serde_json::to_value(&asset.role)
                    .unwrap_or_else(|_| serde_json::to_value(AssetRole::Primary).unwrap()),
            ),
        ]);
        if let Some(stable_key) = &asset.stable_asset_key {
            frozen.insert(
                "stable_asset_key".to_owned(),
                Value::String(stable_key.clone()),
            );
        }
        if let Some(name) = &asset.file_name {
            frozen.insert("file_name".to_owned(), Value::String(name.clone()));
        }
        if let Some(media_type) = &asset.media_type {
            frozen.insert("media_type".to_owned(), Value::String(media_type.clone()));
        }
        if let Some(size) = asset.size_bytes {
            frozen.insert("size_bytes".to_owned(), Value::Number(size.into()));
        }
        if let Some(url) = &asset.url {
            frozen.insert("url".to_owned(), Value::String(url.clone()));
        }
        frozen_assets.push(Value::Object(frozen));
    }

    let metadata = Map::from_iter([
        (
            "logical_dataset".to_owned(),
            Value::String(profile.logical_dataset.to_owned()),
        ),
        (
            "native_collection_id".to_owned(),
            Value::String(profile.native_collection_id.to_owned()),
        ),
        (
            "collection_profile_id".to_owned(),
            Value::String(profile.profile_id.to_owned()),
        ),
        (
            "identity_rule_id".to_owned(),
            Value::String(profile.identity_rule_id.to_owned()),
        ),
        (
            "filename_rule_id".to_owned(),
            Value::String(profile.filename_rule_id.to_owned()),
        ),
        (
            "resolver_policy_id".to_owned(),
            Value::String(profile.resolver_policy_id.to_owned()),
        ),
        (
            "delivery_model".to_owned(),
            serde_json::to_value(profile.delivery_model.clone())
                .expect("closed delivery model serializes"),
        ),
        (
            "provider_product_id".to_owned(),
            Value::String(product.product_id.clone()),
        ),
        (
            "provider_product_key".to_owned(),
            Value::String(item_id.clone()),
        ),
        (
            "source_kind".to_owned(),
            serde_json::to_value(&product.source_kind).expect("closed source kind serializes"),
        ),
        (
            "product_kind".to_owned(),
            Value::String("optical_scene".to_owned()),
        ),
        (
            "properties".to_owned(),
            serde_json::json!({ "platform": platform, "cloud_cover": cloud_cover }),
        ),
        ("assets".to_owned(), Value::Array(frozen_assets)),
    ]);
    validate_debug_context(&metadata).map_err(|_| hls_error("sensitive_metadata"))?;
    let item = SnapshotItem {
        item_id,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: Some(acquisition_datetime),
        footprint_geojson: Some(footprint),
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| hls_error("snapshot"))?;
    if serde_json::to_vec(&item).map_or(true, |bytes| {
        bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES
    }) {
        return Err(hls_error("snapshot_size"));
    }
    Ok(item)
}

/// Issue #3 R2 B5 review H1: a catalogue product is search-only when its
/// Registry resource declares no active delivery capability (HLS v2.0 today).
fn search_only_logical_dataset(logical_dataset: &str) -> Result<bool, ApplicationServiceError> {
    let registry = ResourceRegistry::embedded_ref()
        .map_err(|_| ApplicationServiceError::ResourceRegistryUnavailable)?;
    Ok(registry.resources.iter().any(|resource| match resource {
        ResourceDescriptor::ScienceDataset { common, science } => {
            science.catalogue_logical_dataset.as_deref() == Some(logical_dataset)
                && !common.capabilities.iter().any(|capability| {
                    matches!(
                        capability.kind,
                        CapabilityKind::FullArchive
                            | CapabilityKind::AssetSet
                            | CapabilityKind::Download
                            | CapabilityKind::RemoteSubset
                    ) && !matches!(
                        capability.lifecycle,
                        StaticCapabilityLifecycle::Reserved | StaticCapabilityLifecycle::Retired
                    )
                })
        }
        _ => false,
    }))
}

/// No download task may be created from, or appended out of, a frozen batch
/// of a search-only product, whatever the UI projected (deferred restore,
/// failed page projection or a direct command call). A frozen catalogue batch
/// holds one logical dataset (its query's), so only that value is read.
pub(super) fn reject_search_only_batch(
    store: &TaskStore,
    batch_id: &str,
) -> Result<(), ApplicationServiceError> {
    reject_restricted_sar_batch(store, batch_id)?;
    match store.load_search_batch_logical_dataset(batch_id)? {
        Some(Some(logical_dataset)) => reject_search_only_dataset(&logical_dataset),
        _ => Ok(()),
    }
}

/// #22 B slice 5: the same gate for the frozen query of an admission batch
/// built from arrived rows of a loading batch (no `search_batches` row yet).
pub(super) fn reject_search_only_query(
    query: &Map<String, Value>,
) -> Result<(), ApplicationServiceError> {
    if let Some(dataset) = query.get("dataset").and_then(Value::as_str) {
        reject_restricted_sar_dataset(dataset)?;
    }
    match query.get("logical_dataset").and_then(Value::as_str) {
        Some(logical_dataset) => reject_search_only_dataset(logical_dataset),
        None => Ok(()),
    }
}

/// Defense in depth for entries that start from an existing task (append,
/// source-batch append, historical re-create): the task's frozen query decides.
pub(super) fn reject_search_only_task(
    store: &TaskStore,
    task_id: &str,
) -> Result<(), ApplicationServiceError> {
    reject_restricted_sar_task(store, task_id)?;
    let Some(task) = store.load_task_record(task_id)? else {
        return Ok(());
    };
    match task
        .snapshot
        .query
        .get("logical_dataset")
        .and_then(Value::as_str)
    {
        Some(logical_dataset) => reject_search_only_dataset(logical_dataset),
        None => Ok(()),
    }
}

pub fn is_hls_dataset(logical_dataset: &str) -> bool {
    matches!(logical_dataset, "optical.hls.l30.v2" | "optical.hls.s30.v2")
}

pub const HLS_L30_SUPPORTED_BANDS: &[&str] = &[
    "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B09", "B10", "B11", "Fmask",
];

pub const HLS_S30_SUPPORTED_BANDS: &[&str] = &[
    "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12",
    "Fmask",
];

pub fn validate_hls_band_selection(
    product: MpcHlsProduct,
    asset_keys: &[String],
) -> Result<(), ApplicationServiceError> {
    if asset_keys.is_empty() {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "HLS multi-band selection cannot be empty".to_owned(),
        });
    }
    let supported = match product {
        MpcHlsProduct::L30 => HLS_L30_SUPPORTED_BANDS,
        MpcHlsProduct::S30 => HLS_S30_SUPPORTED_BANDS,
    };
    for key in asset_keys {
        if !supported.contains(&key.as_str()) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: format!("Unsupported band {key} for HLS product"),
            });
        }
    }
    Ok(())
}

pub fn prepare_hls_band_items(
    source_item: &SnapshotItem,
    asset_keys: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<SnapshotItem>, ApplicationServiceError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(ApplicationServiceError::LandsatAssetCancelled);
    }
    let logical_dataset = source_item
        .metadata
        .get("logical_dataset")
        .and_then(Value::as_str)
        .ok_or_else(|| hls_error("logical_dataset"))?;
    let (hls_product, collection_id) = match logical_dataset {
        "optical.hls.l30.v2" => (MpcHlsProduct::L30, "hls2-l30"),
        "optical.hls.s30.v2" => (MpcHlsProduct::S30, "hls2-s30"),
        _ => return Err(hls_error("unknown_hls_product")),
    };
    validate_hls_band_selection(hls_product, asset_keys)?;
    let assets = source_item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| hls_error("assets_missing"))?;
    let mut items = Vec::with_capacity(asset_keys.len());
    for key in asset_keys {
        if cancelled.load(Ordering::Acquire) {
            return Err(ApplicationServiceError::LandsatAssetCancelled);
        }
        let asset_obj = assets
            .iter()
            .find(|item| {
                item.get("asset_key")
                    .and_then(Value::as_str)
                    .is_some_and(|k| k.eq_ignore_ascii_case(key))
            })
            .and_then(Value::as_object)
            .ok_or_else(|| ApplicationServiceError::InvalidRequest {
                reason: format!("HLS band {key} not found in STAC metadata assets"),
            })?;
        let stable_href = asset_obj
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| hls_error("asset_url_missing"))?
            .to_owned();
        let file_name = asset_obj
            .get("file_name")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                Url::parse(&stable_href)
                    .ok()
                    .and_then(|u| {
                        u.path_segments()?
                            .rfind(|s| !s.is_empty())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| format!("{}_{}.tif", source_item.display_name, key))
            });
        let expected_size_bytes = asset_obj.get("size_bytes").and_then(Value::as_u64);
        let role = asset_obj
            .get("role")
            .and_then(|v| serde_json::from_value::<AssetRole>(v.clone()).ok())
            .unwrap_or(AssetRole::Primary);

        let band_item_id = format!("{}-{}", source_item.item_id, key.to_ascii_lowercase());
        let pkey = provider_product_key(
            &source_item.provider_id,
            collection_id,
            &source_item.display_name,
        )
        .map_err(|_| hls_error("provider_product_key"))?;
        let stable_key = stable_asset_key(&pkey, key).map_err(|_| hls_error("stable_asset_key"))?;
        let descriptor = AssetAccessDescriptor {
            resolver_id: "planetary-computer.hls.assets.v1".to_owned(),
            scheme: AssetAccessScheme::PublicHttps,
            requires_credentials: false,
            credential_profile: None,
            stable_href: Some(stable_href),
            resolver_parameters: AssetResolverParameters {},
        };
        let reference = StableAssetReference {
            provider_id: source_item.provider_id.clone(),
            collection_id: collection_id.to_owned(),
            provider_product_id: source_item.display_name.clone(),
            provider_product_key: pkey,
            asset_key: key.clone(),
            stable_asset_key: stable_key,
            canonical_product_key: None,
        };
        let access = FrozenAssetAccess {
            descriptor,
            reference,
            role,
            file_name: Some(file_name.clone()),
            expected_size_bytes,
            checksum_algorithm: None,
            checksum_value: None,
        };
        let mut metadata = source_item.metadata.clone();
        metadata.insert(
            "source_item_id".to_owned(),
            Value::String(source_item.item_id.clone()),
        );
        metadata.insert("delivery_asset_key".to_owned(), Value::String(key.clone()));
        metadata.insert(
            "delivery_asset_role".to_owned(),
            Value::String("primary".to_owned()),
        );
        metadata.insert(
            "delivery_product_directory".to_owned(),
            Value::String(source_item.display_name.clone()),
        );
        let item = SnapshotItem {
            item_id: band_item_id,
            provider_id: source_item.provider_id.clone(),
            display_name: file_name,
            acquisition_datetime: source_item.acquisition_datetime.clone(),
            footprint_geojson: source_item.footprint_geojson.clone(),
            metadata,
            payload: Map::new(),
            asset_access: Some(access),
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: hls_product.asset_policy_id().to_owned(),
                collection_profile_id: Some(hls_product.profile_id().to_owned()),
            }),
        };
        item.validate().map_err(|_| hls_error("snapshot"))?;
        items.push(item);
    }
    Ok(items)
}

fn reject_search_only_dataset(logical_dataset: &str) -> Result<(), ApplicationServiceError> {
    if is_hls_dataset(logical_dataset) {
        return Ok(());
    }
    if search_only_logical_dataset(logical_dataset)? {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: "该数据目前仅支持检索，不能创建下载任务".to_owned(),
        });
    }
    Ok(())
}

/// JERS-1 is a restricted ASF collection: searchable (through the
/// authenticated CMR route) but never downloadable in this product. The
/// Registry's JERS-1 download capability covers the credentialed ASF archive
/// in general, but the mission's research-agreement restriction means no
/// task may be created for it here; downloads that somehow start already
/// fail as `ASF_DATASET_ACCESS_RESTRICTED` in the worker.
const JERS_1_SAR_DATASET: &str = "JERS-1";
const JERS_1_RESTRICTED_REASON_ZH: &str =
    "JERS-1 是受限数据集，仅支持检索，不能创建下载任务。有关数据访问，请联系 ASF 用户支持。";

/// ASF removed ERS, RADARSAT-1 and JERS-1 from its searchable interfaces
/// (Owner evidence 2026-10-31), so their Registry resources are retired and
/// neither search nor a download task can ever succeed for them today. Any
/// frozen batch or task that still carries one of these dataset selectors is
/// rejected with the typed ASF removal reason before any task row exists.
const ASF_REMOVED_LEGACY_SAR_DATASETS: &[&str] = &["ERS", "JERS-1", "RADARSAT-1"];
const ASF_REMOVED_REASON_ZH: &str = "ASF 已从检索接口下线该数据集";

fn asf_removed_legacy_sar_dataset(dataset: &str) -> bool {
    ASF_REMOVED_LEGACY_SAR_DATASETS
        .iter()
        .any(|selector| dataset.trim().eq_ignore_ascii_case(selector))
}

/// Search-side admission for the ASF-removed legacy missions (Owner evidence
/// 2026-10-31): the production provider already rejects them with the typed
/// `ASF_DATASET_REMOVED` reason, and this entry-level gate keeps fixture
/// runtimes and any other LiveAsfSearch seam honest to the same contract
/// before a cache lookup or provider request runs.
pub(super) fn reject_asf_removed_legacy_dataset(
    dataset: &str,
) -> Result<(), ApplicationServiceError> {
    if asf_removed_legacy_sar_dataset(dataset) {
        return Err(ApplicationServiceError::Provider(
            ContractError::new("ASF_DATASET_REMOVED", ASF_REMOVED_REASON_ZH, Map::new())
                .expect("bounded removal context is safe"),
        ));
    }
    Ok(())
}

fn reject_restricted_sar_batch(
    store: &TaskStore,
    batch_id: &str,
) -> Result<(), ApplicationServiceError> {
    if let Some(Some(dataset)) = store.load_search_batch_sar_dataset(batch_id)? {
        if asf_removed_legacy_sar_dataset(&dataset) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: ASF_REMOVED_REASON_ZH.to_owned(),
            });
        }
        if dataset.trim().eq_ignore_ascii_case(JERS_1_SAR_DATASET) {
            return Err(ApplicationServiceError::InvalidRequest {
                reason: JERS_1_RESTRICTED_REASON_ZH.to_owned(),
            });
        }
    }
    Ok(())
}

fn reject_restricted_sar_task(
    store: &TaskStore,
    task_id: &str,
) -> Result<(), ApplicationServiceError> {
    let Some(task) = store.load_task_record(task_id)? else {
        return Ok(());
    };
    if let Some(dataset) = task.snapshot.query.get("dataset").and_then(Value::as_str) {
        reject_restricted_sar_dataset(dataset)?;
    }
    Ok(())
}

fn reject_restricted_sar_dataset(dataset: &str) -> Result<(), ApplicationServiceError> {
    if asf_removed_legacy_sar_dataset(dataset) {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: ASF_REMOVED_REASON_ZH.to_owned(),
        });
    }
    if dataset.trim().eq_ignore_ascii_case(JERS_1_SAR_DATASET) {
        return Err(ApplicationServiceError::InvalidRequest {
            reason: JERS_1_RESTRICTED_REASON_ZH.to_owned(),
        });
    }
    Ok(())
}
