//! Deterministic, validated ASF Burst asset freezing shared by admission and Store.
use crate::{
    stable_asset_key, AssetExecutionAuthority, AssetExecutionPolicy, AssetRole, ProviderAsset,
    SnapshotItem,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use url::Url;

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("invalid Burst asset evidence")]
pub struct BurstAssetEvidenceInvalid;

/// Compare the complete executable asset set with a fresh deterministic
/// expansion of the authoritative parent rows and the existing task policy.
pub fn validate_burst_task_items(
    parents: &[SnapshotItem],
    existing: &[SnapshotItem],
    proposed: &[SnapshotItem],
) -> Result<(), BurstAssetEvidenceInvalid> {
    let policy = frozen_burst_task_policy(existing)?;
    validate_burst_task_items_for_policy(parents, policy, proposed)
}

pub fn validate_burst_task_items_for_policy(
    parents: &[SnapshotItem],
    policy: BurstAssetPolicy,
    proposed: &[SnapshotItem],
) -> Result<(), BurstAssetEvidenceInvalid> {
    let refs = parents.iter().collect::<Vec<_>>();
    let expected = freeze_burst_task_items(&refs, policy)?;
    let proposed_by_id = proposed
        .iter()
        .map(|item| (item.item_id.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    if proposed_by_id.len() != proposed.len()
        || expected.len() != proposed.len()
        || expected
            .iter()
            .any(|item| proposed_by_id.get(item.item_id.as_str()).copied() != Some(item))
    {
        return Err(BurstAssetEvidenceInvalid);
    }
    Ok(())
}

pub const ASF_BURST_ASSET_EXECUTION_POLICY_ID: &str = "asf.sentinel-1.burst.asset.v1";
pub const ASF_BURST_COLLECTION_PROFILE_ID: &str = "asf.sentinel-1.burst.v1";
const ASF_BURST_ASSET_HOST: &str = "sentinel1-burst.asf.alaska.edu";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BurstAssetPolicy {
    All,
    Primary,
    Metadata,
}

impl BurstAssetPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Primary => "primary",
            Self::Metadata => "metadata",
        }
    }

    const fn includes(self, role: &AssetRole) -> bool {
        matches!(
            (self, role),
            (Self::All, AssetRole::Primary | AssetRole::Metadata)
                | (Self::Primary, AssetRole::Primary)
                | (Self::Metadata, AssetRole::Metadata)
        )
    }
}

pub fn freeze_burst_task_items(
    source_products: &[&SnapshotItem],
    policy: BurstAssetPolicy,
) -> Result<Vec<SnapshotItem>, BurstAssetEvidenceInvalid> {
    let mut items = Vec::new();
    let mut stable_keys = BTreeSet::new();
    for source in source_products {
        if source.provider_id != "asf.catalog"
            || source
                .metadata
                .get("dataset")
                .and_then(Value::as_str)
                .is_none_or(|dataset| !dataset.eq_ignore_ascii_case("SLC-BURST"))
            || source
                .metadata
                .get("product_family")
                .and_then(Value::as_str)
                .is_none_or(|family| !family.eq_ignore_ascii_case("BURST"))
        {
            return Err(BurstAssetEvidenceInvalid);
        }
        let assets = source
            .metadata
            .get("assets")
            .cloned()
            .and_then(|value| serde_json::from_value::<Vec<ProviderAsset>>(value).ok())
            .ok_or(BurstAssetEvidenceInvalid)?;
        if assets.len() != 2 {
            return Err(BurstAssetEvidenceInvalid);
        }
        let primary = unique_burst_asset(&assets, &AssetRole::Primary)?;
        let metadata = unique_burst_asset(&assets, &AssetRole::Metadata)?;
        for asset in [primary, metadata] {
            validate_frozen_burst_asset(source, asset)?;
            if !policy.includes(&asset.role) {
                continue;
            }
            let stable_asset_key = asset
                .stable_asset_key
                .as_deref()
                .ok_or(BurstAssetEvidenceInvalid)?;
            if !stable_keys.insert(stable_asset_key.to_owned()) {
                return Err(BurstAssetEvidenceInvalid);
            }
            let official_href = asset.url.as_deref().ok_or(BurstAssetEvidenceInvalid)?;
            let file_name = asset
                .file_name
                .as_deref()
                .ok_or(BurstAssetEvidenceInvalid)?;
            let media_type = asset
                .media_type
                .as_deref()
                .ok_or(BurstAssetEvidenceInvalid)?;
            let role = burst_asset_role_name(&asset.role)?;

            let mut frozen_metadata = source.metadata.clone();
            frozen_metadata.remove("assets");
            frozen_metadata.insert(
                "source_item_id".to_owned(),
                Value::String(source.item_id.clone()),
            );
            frozen_metadata.insert(
                "stable_asset_key".to_owned(),
                Value::String(stable_asset_key.to_owned()),
            );
            frozen_metadata.insert("asset_role".to_owned(), Value::String(role.to_owned()));
            frozen_metadata.insert(
                "official_href".to_owned(),
                Value::String(official_href.to_owned()),
            );
            frozen_metadata.insert(
                "asset_basename".to_owned(),
                Value::String(file_name.to_owned()),
            );
            frozen_metadata.insert(
                "media_type".to_owned(),
                Value::String(media_type.to_owned()),
            );
            frozen_metadata.insert(
                "size_bytes".to_owned(),
                asset.size_bytes.map(Value::from).unwrap_or(Value::Null),
            );
            frozen_metadata.insert(
                "burst_asset_policy".to_owned(),
                Value::String(policy.as_str().to_owned()),
            );

            let mut payload = Map::from_iter([
                ("burst_asset".to_owned(), Value::Bool(true)),
                (
                    "download_url".to_owned(),
                    Value::String(official_href.to_owned()),
                ),
                ("file_name".to_owned(), Value::String(file_name.to_owned())),
                (
                    "media_type".to_owned(),
                    Value::String(media_type.to_owned()),
                ),
                (
                    "source_item_id".to_owned(),
                    Value::String(source.item_id.clone()),
                ),
                ("asset_role".to_owned(), Value::String(role.to_owned())),
                (
                    "burst_asset_policy".to_owned(),
                    Value::String(policy.as_str().to_owned()),
                ),
            ]);
            if let Some(size_bytes) = asset.size_bytes {
                payload.insert("file_size_remote".to_owned(), Value::from(size_bytes));
            }
            items.push(SnapshotItem {
                item_id: stable_asset_key.to_owned(),
                provider_id: source.provider_id.clone(),
                display_name: file_name.to_owned(),
                acquisition_datetime: source.acquisition_datetime.clone(),
                footprint_geojson: source.footprint_geojson.clone(),
                metadata: frozen_metadata,
                payload,
                asset_access: None,
                execution_policy: Some(AssetExecutionPolicy {
                    authority: AssetExecutionAuthority::ResolverAuthoritative,
                    policy_id: ASF_BURST_ASSET_EXECUTION_POLICY_ID.to_owned(),
                    collection_profile_id: Some(ASF_BURST_COLLECTION_PROFILE_ID.to_owned()),
                }),
            });
        }
    }
    if items.is_empty() {
        return Err(BurstAssetEvidenceInvalid);
    }
    Ok(items)
}

pub fn frozen_burst_task_policy(
    task_items: &[SnapshotItem],
) -> Result<BurstAssetPolicy, BurstAssetEvidenceInvalid> {
    let Some(first) = task_items
        .first()
        .and_then(|item| item.metadata.get("burst_asset_policy"))
        .and_then(Value::as_str)
    else {
        return Err(BurstAssetEvidenceInvalid);
    };
    if task_items.iter().any(|item| {
        item.metadata
            .get("burst_asset_policy")
            .and_then(Value::as_str)
            != Some(first)
    }) {
        return Err(BurstAssetEvidenceInvalid);
    }
    match first {
        "all" => Ok(BurstAssetPolicy::All),
        "primary" => Ok(BurstAssetPolicy::Primary),
        "metadata" => Ok(BurstAssetPolicy::Metadata),
        _ => Err(BurstAssetEvidenceInvalid),
    }
}

fn unique_burst_asset<'a>(
    assets: &'a [ProviderAsset],
    role: &AssetRole,
) -> Result<&'a ProviderAsset, BurstAssetEvidenceInvalid> {
    let mut matching = assets.iter().filter(|asset| &asset.role == role);
    let asset = matching.next().ok_or(BurstAssetEvidenceInvalid)?;
    if matching.next().is_some() {
        return Err(BurstAssetEvidenceInvalid);
    }
    Ok(asset)
}

fn validate_frozen_burst_asset(
    source: &SnapshotItem,
    asset: &ProviderAsset,
) -> Result<(), BurstAssetEvidenceInvalid> {
    asset.validate().map_err(|_| BurstAssetEvidenceInvalid)?;
    let asset_key = asset
        .asset_key
        .as_deref()
        .ok_or(BurstAssetEvidenceInvalid)?;
    let frozen_stable_key = asset
        .stable_asset_key
        .as_deref()
        .ok_or(BurstAssetEvidenceInvalid)?;
    let expected_stable_key =
        stable_asset_key(&source.item_id, asset_key).map_err(|_| BurstAssetEvidenceInvalid)?;
    if frozen_stable_key != expected_stable_key
        || asset.access_descriptor.is_some()
        || asset.native_roles.as_slice() != ["USE SERVICE API"]
        || asset.size_bytes.is_some_and(|size| size < 0)
    {
        return Err(BurstAssetEvidenceInvalid);
    }
    let official_href = asset.url.as_deref().ok_or(BurstAssetEvidenceInvalid)?;
    let file_name = asset
        .file_name
        .as_deref()
        .ok_or(BurstAssetEvidenceInvalid)?;
    let media_type = asset
        .media_type
        .as_deref()
        .ok_or(BurstAssetEvidenceInvalid)?;
    if official_href
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
        || file_name.is_empty()
        || file_name.len() > 255
        || file_name.contains(['/', '\\', '%'])
        || file_name.chars().any(char::is_control)
        || file_name.chars().any(char::is_whitespace)
    {
        return Err(BurstAssetEvidenceInvalid);
    }
    let parsed = Url::parse(official_href).map_err(|_| BurstAssetEvidenceInvalid)?;
    let url_file_name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .ok_or(BurstAssetEvidenceInvalid)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(ASF_BURST_ASSET_HOST)
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path().contains('%')
        || url_file_name != file_name
    {
        return Err(BurstAssetEvidenceInvalid);
    }
    match (&asset.role, media_type) {
        (AssetRole::Primary, "image/tiff")
            if file_name.ends_with(".tiff") && asset.size_bytes.is_some_and(|size| size > 0) => {}
        (AssetRole::Metadata, "application/xml") if file_name.ends_with(".xml") => {}
        _ => return Err(BurstAssetEvidenceInvalid),
    }
    Ok(())
}

fn burst_asset_role_name(role: &AssetRole) -> Result<&'static str, BurstAssetEvidenceInvalid> {
    match role {
        AssetRole::Primary => Ok("primary"),
        AssetRole::Metadata => Ok("metadata"),
        _ => Err(BurstAssetEvidenceInvalid),
    }
}
