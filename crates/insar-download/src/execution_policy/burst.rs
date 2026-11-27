use insar_core::{
    stable_asset_key, AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, SnapshotItem, StableAssetReference,
};
use reqwest::Url;
use serde_json::Value;

use super::{
    AssetExecutionPolicyDefinition, AuthRefreshPolicyId, EligibleResolverAsset,
    ExecutionDeliveryModel, ExecutionPolicyError, ResolverCutoverEligibility,
};
use insar_provider::asf::collection_profile::AsfDeliveryModel;

pub const ASF_BURST_ASSET_EXECUTION_POLICY_ID: &str = "asf.sentinel-1.burst.asset.v1";
pub const ASF_BURST_COLLECTION_PROFILE_ID: &str = "asf.sentinel-1.burst.v1";
pub const EARTHDATA_BURST_CREDENTIAL_PROFILE: &str = "earthdata-burst-session";
const ASF_BURST_PROVIDER_ID: &str = "asf.catalog";
const ASF_BURST_COLLECTION_ID: &str = "SLC-BURST";
const ASF_BURST_ASSET_HOST: &str = "sentinel1-burst.asf.alaska.edu";

fn metadata_text<'a>(item: &'a SnapshotItem, key: &'static str) -> Option<&'a str> {
    item.metadata.get(key).and_then(Value::as_str)
}

fn payload_text<'a>(item: &'a SnapshotItem, key: &'static str) -> Option<&'a str> {
    item.payload.get(key).and_then(Value::as_str)
}

fn invalid_access(reason: &'static str) -> ExecutionPolicyError {
    ExecutionPolicyError::InvalidAssetAccess(reason.to_owned())
}

fn role_from_evidence(value: &str) -> Result<AssetRole, ExecutionPolicyError> {
    match value {
        "primary" => Ok(AssetRole::Primary),
        "metadata" => Ok(AssetRole::Metadata),
        _ => Err(invalid_access("Burst asset role is invalid")),
    }
}

fn valid_cmr_granule_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn matching_required_text(left: Option<&str>, right: Option<&str>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if !left.is_empty() && left == right)
}

fn matching_optional_text(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => !left.is_empty() && left == right,
        _ => false,
    }
}

fn matching_positive_i64(left: Option<i64>, right: Option<i64>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left > 0 && left == right)
}

fn matching_non_negative_i64(left: Option<i64>, right: Option<i64>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left >= 0 && left == right)
}

fn validate_burst_parent_evidence(
    item: &SnapshotItem,
    source_item_id: &str,
    provider_product_id: &str,
    native_product_id: &str,
) -> Result<u64, ExecutionPolicyError> {
    let expected_source_item_id = format!("ASF::SLC-BURST::{native_product_id}");
    let stable_asset_keys = item
        .metadata
        .get("stable_asset_keys")
        .and_then(Value::as_array)
        .ok_or(ExecutionPolicyError::MissingIdentity("stable_asset_keys"))?;
    let native = item
        .metadata
        .get("native_metadata")
        .and_then(Value::as_object)
        .ok_or(ExecutionPolicyError::MissingIdentity("native_metadata"))?;
    let collection = native
        .get("collection")
        .and_then(Value::as_object)
        .ok_or(ExecutionPolicyError::MissingIdentity("native_collection"))?;
    let native_burst = native
        .get("burst")
        .and_then(Value::as_object)
        .ok_or(ExecutionPolicyError::MissingIdentity("native_burst"))?;
    let native_scope = native
        .get("scope")
        .and_then(Value::as_object)
        .ok_or(ExecutionPolicyError::MissingIdentity("native_scope"))?;
    let burst = item
        .metadata
        .get("burst")
        .and_then(Value::as_object)
        .ok_or(ExecutionPolicyError::MissingIdentity("burst"))?;
    let primary_size_bytes = native_burst
        .get("byte_length")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or(ExecutionPolicyError::MissingIdentity("burst_byte_length"))?;

    let stable_keys_are_closed = stable_asset_keys.len() == 2
        && stable_asset_keys.iter().all(Value::is_string)
        && stable_asset_keys[0] != stable_asset_keys[1]
        && stable_asset_keys
            .iter()
            .any(|value| value.as_str() == Some(item.item_id.as_str()));
    let native_identity_matches = source_item_id == expected_source_item_id
        && metadata_text(item, "scene_id") == Some(native_product_id)
        && valid_cmr_granule_concept_id(provider_product_id)
        && native.get("cmr_native_id").and_then(Value::as_str) == Some(native_product_id)
        && native.get("cmr_granule_concept_id").and_then(Value::as_str)
            == Some(provider_product_id)
        && native.get("raw_authority").is_some_and(Value::is_object)
        && native
            .get("preview_available")
            .is_some_and(Value::is_boolean);
    let collection_matches = collection.get("concept_id").and_then(Value::as_str)
        == Some("C2709161906-ASF")
        && collection.get("short_name").and_then(Value::as_str) == Some("SENTINEL-1_BURSTS")
        && collection.get("version").and_then(Value::as_str) == Some("1")
        && collection.get("provider_id").and_then(Value::as_str) == Some("ASF");
    let burst_identity_matches = matching_required_text(
        burst.get("full_burst_id").and_then(Value::as_str),
        native_burst.get("full_id").and_then(Value::as_str),
    ) && matching_positive_i64(
        burst.get("relative_burst_id").and_then(Value::as_i64),
        native_burst.get("relative_id").and_then(Value::as_i64),
    ) && matching_positive_i64(
        burst.get("absolute_burst_id").and_then(Value::as_i64),
        native_burst.get("absolute_id").and_then(Value::as_i64),
    ) && matching_non_negative_i64(
        burst.get("burst_index").and_then(Value::as_i64),
        native_burst.get("index").and_then(Value::as_i64),
    ) && matching_required_text(
        burst.get("subswath").and_then(Value::as_str),
        native_burst.get("subswath").and_then(Value::as_str),
    );
    let scope_matches = matching_required_text(
        metadata_text(item, "beam_mode"),
        native_scope.get("beam_mode").and_then(Value::as_str),
    ) && matching_required_text(
        metadata_text(item, "beam_swath"),
        native_burst.get("subswath").and_then(Value::as_str),
    ) && matching_required_text(
        metadata_text(item, "flight_direction"),
        native_scope.get("flight_direction").and_then(Value::as_str),
    ) && matching_required_text(
        metadata_text(item, "platform"),
        native_scope.get("platform").and_then(Value::as_str),
    ) && matching_required_text(
        metadata_text(item, "instrument"),
        native_scope.get("instrument").and_then(Value::as_str),
    ) && matching_optional_text(
        // ASF's Sentinel-1D burst records carry no LookDirection: both sides
        // then record none; one side alone is still a mismatch.
        metadata_text(item, "look_direction"),
        native_scope.get("look_direction").and_then(Value::as_str),
    ) && matching_positive_i64(
        item.metadata.get("path").and_then(Value::as_i64),
        native_scope.get("path").and_then(Value::as_i64),
    ) && native_scope.get("processing_level").and_then(Value::as_str)
        == Some("L1")
        && native_scope.get("processing_type").and_then(Value::as_str) == Some("BURST")
        && matches!(
            metadata_text(item, "platform"),
            Some("SENTINEL-1A" | "SENTINEL-1B" | "SENTINEL-1C" | "SENTINEL-1D")
        )
        && metadata_text(item, "instrument") == Some("C-SAR");
    if !stable_keys_are_closed
        || !native_identity_matches
        || !collection_matches
        || !burst_identity_matches
        || !scope_matches
    {
        return Err(ExecutionPolicyError::IdentityMismatch(
            "burst_parent_authority",
        ));
    }
    Ok(primary_size_bytes)
}

pub fn build_burst_frozen_asset_access(
    item: &SnapshotItem,
) -> Result<FrozenAssetAccess, ExecutionPolicyError> {
    item.validate().map_err(ExecutionPolicyError::Structural)?;
    let policy = item
        .execution_policy
        .as_ref()
        .ok_or(ExecutionPolicyError::MissingIdentity("execution_policy"))?;
    if policy.authority != AssetExecutionAuthority::ResolverAuthoritative
        || policy.policy_id != ASF_BURST_ASSET_EXECUTION_POLICY_ID
        || policy.collection_profile_id.as_deref() != Some(ASF_BURST_COLLECTION_PROFILE_ID)
    {
        return Err(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != ASF_BURST_PROVIDER_ID
        || metadata_text(item, "provider") != Some(ASF_BURST_PROVIDER_ID)
        || metadata_text(item, "dataset") != Some(ASF_BURST_COLLECTION_ID)
        || metadata_text(item, "product_family") != Some("BURST")
        || metadata_text(item, "processing_level") != Some("L1")
        || metadata_text(item, "identity_rule_id") != Some(ASF_BURST_COLLECTION_PROFILE_ID)
        || item.asset_access.is_some()
        || item.payload.get("burst_asset").and_then(Value::as_bool) != Some(true)
    {
        return Err(ExecutionPolicyError::IdentityMismatch("burst_item"));
    }

    let source_item_id = metadata_text(item, "source_item_id")
        .ok_or(ExecutionPolicyError::MissingIdentity("source_item_id"))?;
    let product_key = metadata_text(item, "product_key")
        .ok_or(ExecutionPolicyError::MissingIdentity("product_key"))?;
    let provider_product_key = metadata_text(item, "provider_product_key").ok_or(
        ExecutionPolicyError::MissingIdentity("provider_product_key"),
    )?;
    if source_item_id != product_key
        || source_item_id != provider_product_key
        || payload_text(item, "source_item_id") != Some(source_item_id)
    {
        return Err(ExecutionPolicyError::IdentityMismatch("source_item_id"));
    }
    let provider_product_id = metadata_text(item, "provider_product_id")
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExecutionPolicyError::MissingIdentity("provider_product_id"))?;
    let native_product_id = metadata_text(item, "product_id")
        .filter(|value| !value.trim().is_empty())
        .ok_or(ExecutionPolicyError::MissingIdentity("product_id"))?;
    let parent_primary_size_bytes = validate_burst_parent_evidence(
        item,
        source_item_id,
        provider_product_id,
        native_product_id,
    )?;
    let role_name = metadata_text(item, "asset_role")
        .ok_or(ExecutionPolicyError::MissingIdentity("asset_role"))?;
    let role = role_from_evidence(role_name)?;
    let file_name = metadata_text(item, "asset_basename")
        .filter(|value| super::safe_basename(value) && !value.contains('%'))
        .ok_or(ExecutionPolicyError::InvalidFilename)?;
    let media_type = metadata_text(item, "media_type")
        .ok_or(ExecutionPolicyError::MissingIdentity("media_type"))?;
    let asset_policy = metadata_text(item, "burst_asset_policy")
        .ok_or(ExecutionPolicyError::MissingIdentity("burst_asset_policy"))?;
    if payload_text(item, "asset_role") != Some(role_name)
        || payload_text(item, "file_name") != Some(file_name)
        || payload_text(item, "media_type") != Some(media_type)
        || payload_text(item, "burst_asset_policy") != Some(asset_policy)
        || item.display_name != file_name
        || !matches!(
            (role.clone(), asset_policy),
            (AssetRole::Primary, "all" | "primary") | (AssetRole::Metadata, "all" | "metadata")
        )
    {
        return Err(ExecutionPolicyError::IdentityMismatch("asset_evidence"));
    }

    let expected_size_bytes = item.metadata.get("size_bytes").and_then(Value::as_u64);
    match role {
        AssetRole::Primary
            if media_type == "image/tiff"
                && file_name.ends_with(".tiff")
                && expected_size_bytes.is_some_and(|size| size > 0)
                && expected_size_bytes == Some(parent_primary_size_bytes)
                && item.payload.get("file_size_remote").and_then(Value::as_u64)
                    == expected_size_bytes => {}
        AssetRole::Metadata
            if media_type == "application/xml"
                && file_name.ends_with(".xml")
                && expected_size_bytes.is_none()
                && !item.payload.contains_key("file_size_remote") => {}
        _ => {
            return Err(invalid_access(
                "Burst role media or size evidence is invalid",
            ))
        }
    }

    let official_href = metadata_text(item, "official_href")
        .ok_or(ExecutionPolicyError::MissingIdentity("official_href"))?;
    if payload_text(item, "download_url") != Some(official_href) {
        return Err(ExecutionPolicyError::IdentityMismatch("official_href"));
    }
    let parsed = Url::parse(official_href).map_err(|_| invalid_access("Burst href is invalid"))?;
    let href_name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .ok_or_else(|| invalid_access("Burst href filename is missing"))?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(ASF_BURST_ASSET_HOST)
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path().contains('%')
        || href_name != file_name
    {
        return Err(invalid_access(
            "Burst href is not an approved durable asset locator",
        ));
    }

    let asset_key = format!("{native_product_id}:{file_name}");
    let expected_stable_key = stable_asset_key(source_item_id, &asset_key)
        .map_err(|_| ExecutionPolicyError::InvalidStableAssetKey)?;
    if item.item_id != expected_stable_key
        || metadata_text(item, "stable_asset_key") != Some(expected_stable_key.as_str())
    {
        return Err(ExecutionPolicyError::InvalidStableAssetKey);
    }

    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: ASF_BURST_PROVIDER_ID.to_owned(),
            collection_id: ASF_BURST_COLLECTION_ID.to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: source_item_id.to_owned(),
            canonical_product_key: metadata_text(item, "canonical_product_key").map(str::to_owned),
            asset_key,
            stable_asset_key: expected_stable_key,
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: "earthdata_https".to_owned(),
            scheme: AssetAccessScheme::EarthdataHttps,
            stable_href: Some(official_href.to_owned()),
            requires_credentials: true,
            credential_profile: Some(EARTHDATA_BURST_CREDENTIAL_PROFILE.to_owned()),
            resolver_parameters: AssetResolverParameters {},
        },
        role,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes,
        checksum_algorithm: None,
        checksum_value: None,
    };
    access
        .validate()
        .map_err(ExecutionPolicyError::InvalidAssetAccess)?;
    Ok(access)
}

pub(super) fn evaluate_burst_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    if policy.policy_id != ASF_BURST_ASSET_EXECUTION_POLICY_ID
        || policy.collection_profile_id != ASF_BURST_COLLECTION_PROFILE_ID
        || policy.delivery_model != ExecutionDeliveryModel::Asf(AsfDeliveryModel::MultiAssetProduct)
        || policy.approved_product_families != ["BURST"]
        || policy.approved_processing_levels != ["L1"]
        || policy.approved_resolver_ids != ["earthdata_https"]
        || policy.approved_credential_profiles != [EARTHDATA_BURST_CREDENTIAL_PROFILE]
        || policy.approved_asset_roles != [AssetRole::Primary, AssetRole::Metadata]
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::EarthdataBurstSessionV1
    {
        return ResolverCutoverEligibility::Invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    match build_burst_frozen_asset_access(item) {
        Ok(frozen_access) => {
            ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
                frozen_access,
                policy,
                artifact_validation_plan: None,
            }))
        }
        Err(error) => ResolverCutoverEligibility::Invalid(error),
    }
}
