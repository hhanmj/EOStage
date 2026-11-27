//! Offline-only mapping; the normal dataset mapper and Registry stay closed.

use super::*;
use insar_provider::catalog::StacOfflineFixtureAdmission;

pub(super) fn map_product(
    admission: &StacOfflineFixtureAdmission,
    product: ProviderProduct,
    route_use: RouteUse,
) -> Result<SnapshotItem, ApplicationServiceError> {
    admission
        .validate_product(&product, route_use)
        .map_err(ApplicationServiceError::Provider)?;
    let binding = admission.binding();
    let source_kind = serde_json::to_value(&product.source_kind).map_err(|_| {
        catalogue_provider_error(
            "STAC_RESPONSE_INVALID",
            "测试目录产品类型无效",
            "fixture_source_kind",
        )
    })?;
    let product_kind = serde_json::to_value(&product.product_kind).map_err(|_| {
        catalogue_provider_error(
            "STAC_RESPONSE_INVALID",
            "测试目录产品类型无效",
            "fixture_product_kind",
        )
    })?;
    let key = provider_product_key(
        &product.provider_id,
        &binding.native_collection_id,
        &product.product_id,
    )
    .map_err(|_| {
        catalogue_provider_error(
            "STAC_IDENTITY_MISMATCH",
            "测试目录产品身份无效",
            "fixture_product_key",
        )
    })?;
    // Only descriptive asset facts are frozen. Even an unsigned URL is not
    // copied into payload/metadata as a legacy transfer or preview escape hatch.
    let assets = product
        .assets
        .iter()
        .map(|asset| {
            serde_json::json!({
                "asset_key": asset.asset_key,
                "stable_asset_key": asset.stable_asset_key,
                "role": asset.role,
                "native_roles": asset.native_roles,
                "file_name": asset.file_name,
                "media_type": asset.media_type,
                "size_bytes": asset.size_bytes,
            })
        })
        .collect::<Vec<_>>();
    let metadata = serde_json::json!({
        "logical_dataset": binding.logical_dataset,
        "native_collection_id": binding.native_collection_id,
        "collection_profile_id": binding.collection_profile_id,
        "identity_rule_id": binding.identity_rule_id,
        "source_kind": source_kind,
        "product_kind": product_kind,
        "provider_product_id": product.product_id,
        "stac_item_id": product.properties.get("stac_item_id"),
        "stac_properties": product.properties.get("stac_properties"),
        "assets": assets,
        "offline_fixture_only": true,
    })
    .as_object()
    .cloned()
    .expect("fixed fixture metadata object");
    let item = SnapshotItem {
        item_id: key,
        provider_id: product.provider_id,
        display_name: product.display_name,
        acquisition_datetime: product.acquisition_datetime,
        footprint_geojson: product.footprint_geojson,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    item.validate().map_err(|_| {
        catalogue_provider_error(
            "STAC_RESPONSE_INVALID",
            "测试目录快照无效",
            "fixture_snapshot",
        )
    })?;
    if serde_json::to_vec(&item).map_or(true, |bytes| {
        bytes.len() > MAX_CATALOGUE_CANDIDATE_JSON_BYTES
    }) {
        return Err(catalogue_provider_error(
            "STAC_RESPONSE_INVALID",
            "测试目录快照超出限制",
            "fixture_snapshot_size",
        ));
    }
    Ok(item)
}
