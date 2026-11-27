use std::collections::{HashMap, HashSet};

use insar_core::{AsfRawFeature, ContractError, ProviderAsset, SarProduct};
use serde_json::{Map, Value};

use super::collection_profile::profile_for_dataset;
use super::metadata_mapper::map_sar_feature;

#[derive(Debug)]
pub struct AsfMappedBatch {
    pub products: Vec<SarProduct>,
    pub item_errors: Vec<ContractError>,
    pub fetched_record_count: u64,
    pub replayed_record_count: u64,
    pub mapped_record_count: u64,
    pub mapping_error_count: u64,
    pub aggregated_asset_record_count: u64,
    pub unique_provider_product_count: u64,
    pub asset_count: u64,
    pub identity_conflict_count: u64,
}

fn remote_record_key(feature: &AsfRawFeature) -> String {
    serde_json::to_string(feature).unwrap_or_else(|_| format!("{feature:?}"))
}

fn native_property(product: &SarProduct, name: &str) -> Option<String> {
    let properties = product
        .native_metadata
        .get("properties")
        .and_then(Value::as_object)?;
    properties.get(name).and_then(|value| match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn conflicting_optional(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left != right)
}

fn product_conflict_reason(left: &SarProduct, right: &SarProduct) -> Option<&'static str> {
    if left.provider_product_id != right.provider_product_id {
        return Some("provider_product_id");
    }
    if left.identity_rule_id != right.identity_rule_id {
        return Some("identity_rule_id");
    }
    if conflicting_optional(
        left.processing_level.as_deref(),
        right.processing_level.as_deref(),
    ) {
        return Some("processing_level");
    }
    if conflicting_optional(left.platform.as_deref(), right.platform.as_deref()) {
        return Some("platform");
    }
    if conflicting_optional(
        left.acquisition_start.as_deref(),
        right.acquisition_start.as_deref(),
    ) {
        return Some("acquisition_start");
    }
    if let Some(profile) = profile_for_dataset(&left.dataset) {
        for field in profile.critical_identity_fields {
            if conflicting_optional(
                native_property(left, field).as_deref(),
                native_property(right, field).as_deref(),
            ) {
                return Some("collection_critical_metadata");
            }
        }
    }
    None
}

fn merge_asset(target: &mut ProviderAsset, incoming: ProviderAsset) -> Result<(), &'static str> {
    if conflicting_optional(target.file_name.as_deref(), incoming.file_name.as_deref()) {
        return Err("asset_file_name");
    }
    if target.url.is_none() {
        target.url = incoming.url;
    }
    if target.file_name.is_none() {
        target.file_name = incoming.file_name;
    }
    if target.media_type.is_none() {
        target.media_type = incoming.media_type;
    }
    if target.size_bytes.is_none() {
        target.size_bytes = incoming.size_bytes;
    }
    if target.access_descriptor.is_none() {
        target.access_descriptor = incoming.access_descriptor;
    } else if incoming.access_descriptor.is_some()
        && target.access_descriptor != incoming.access_descriptor
    {
        return Err("asset_access_descriptor");
    }
    Ok(())
}

fn merge_product_assets(target: &mut SarProduct, incoming: SarProduct) -> Result<(), &'static str> {
    if let Some(reason) = product_conflict_reason(target, &incoming) {
        return Err(reason);
    }
    for asset in incoming.assets {
        let stable_key = asset
            .stable_asset_key
            .as_deref()
            .ok_or("missing_stable_asset_key")?;
        if let Some(existing) = target
            .assets
            .iter_mut()
            .find(|candidate| candidate.stable_asset_key.as_deref() == Some(stable_key))
        {
            merge_asset(existing, asset)?;
        } else {
            target.assets.push(asset);
        }
    }
    Ok(())
}

fn identity_conflict(product_key: &str, reason: &str) -> ContractError {
    ContractError::new(
        "ASF_IDENTITY_CONFLICT",
        "ASF 产品身份存在冲突，冲突记录未加入结果",
        Map::from_iter([
            (
                "product_key".to_string(),
                Value::String(product_key.to_string()),
            ),
            ("reason".to_string(), Value::String(reason.to_string())),
        ]),
    )
    .expect("bounded identity conflict context is safe")
}

pub fn map_and_aggregate_sar_features(dataset: &str, features: &[AsfRawFeature]) -> AsfMappedBatch {
    let fetched_record_count = features.len() as u64;
    let mut seen_records = HashSet::new();
    let mut replayed_record_count = 0;
    let mut mapped_record_count = 0;
    let mut aggregated_asset_record_count = 0;
    let mut identity_conflict_count = 0;
    let mut item_errors = Vec::new();
    let mut products: Vec<Option<SarProduct>> = Vec::new();
    let mut product_indices: HashMap<String, usize> = HashMap::new();
    let mut conflicted_keys = HashSet::new();

    for feature in features {
        if !seen_records.insert(remote_record_key(feature)) {
            replayed_record_count += 1;
            continue;
        }
        let product = match map_sar_feature(dataset, feature) {
            Ok(product) => product,
            Err(error) => {
                item_errors.push(error);
                continue;
            }
        };
        mapped_record_count += 1;
        aggregated_asset_record_count += product.assets.len() as u64;
        let product_key = product.product_key.clone();
        if conflicted_keys.contains(&product_key) {
            continue;
        }
        if let Some(index) = product_indices.get(&product_key).copied() {
            let Some(existing) = products[index].as_mut() else {
                continue;
            };
            if let Err(reason) = merge_product_assets(existing, product) {
                products[index] = None;
                conflicted_keys.insert(product_key.clone());
                identity_conflict_count += 1;
                item_errors.push(identity_conflict(&product_key, reason));
            }
        } else {
            product_indices.insert(product_key, products.len());
            products.push(Some(product));
        }
    }

    let products = products.into_iter().flatten().collect::<Vec<_>>();
    let asset_count = products
        .iter()
        .map(|product| product.assets.len() as u64)
        .sum();
    AsfMappedBatch {
        unique_provider_product_count: products.len() as u64,
        products,
        mapping_error_count: item_errors.len() as u64,
        item_errors,
        fetched_record_count,
        replayed_record_count,
        mapped_record_count,
        aggregated_asset_record_count,
        asset_count,
        identity_conflict_count,
    }
}
