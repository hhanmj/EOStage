use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessScheme, AssetRole, FrozenAssetAccess,
    SnapshotItem,
};
use insar_provider::catalog::{
    alos2_expected_polarizations, CollectionProfileRegistry, ALOS2_COLLECTION_SHORT_NAME,
    ALOS2_COLLECTION_VERSION, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    ALOS2_RESOLVER_POLICY_ID,
};
use serde_json::Value;

use super::{
    invalid, metadata_text, safe_basename, AssetExecutionPolicyDefinition, AuthRefreshPolicyId,
    EligibilityReason, EligibleResolverAsset, ExecutionDeliveryModel, ExecutionPolicyError,
    ResolverCutoverEligibility,
};

pub(super) static ASF_ALOS2_COMPLETE_ZIP_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: ALOS2_RESOLVER_POLICY_ID,
        collection_profile_id: ALOS2_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::Alos2ArchiveFirst,
        approved_product_families: &[ALOS2_COLLECTION_SHORT_NAME],
        approved_processing_levels: &["1.1"],
        approved_resolver_ids: &["earthdata_https"],
        approved_credential_profiles: &["earthdata-default"],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::EarthdataStandardV1,
        artifact_validator_binding: None,
    };

pub(super) fn evaluate_alos2_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let Some(profile) = CollectionProfileRegistry.get(ALOS2_PROFILE_ID) else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    if policy.policy_id != ALOS2_RESOLVER_POLICY_ID
        || policy.collection_profile_id != ALOS2_PROFILE_ID
        || policy.delivery_model != ExecutionDeliveryModel::Alos2ArchiveFirst
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::EarthdataStandardV1
        || profile.provider_id != ALOS2_PROVIDER_ID
        || profile.native_collection_id != ALOS2_NATIVE_COLLECTION_ID
        || profile.resolver_policy_id != ALOS2_RESOLVER_POLICY_ID
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != ALOS2_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "dataset") != Some(ALOS2_LOGICAL_DATASET)
        || metadata_text(item, "logical_dataset") != Some(ALOS2_LOGICAL_DATASET)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(ALOS2_NATIVE_COLLECTION_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(ALOS2_PROFILE_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(ALOS2_IDENTITY_RULE_ID)
        || metadata_text(item, "filename_rule_id") != Some(ALOS2_FILENAME_RULE_ID)
        || metadata_text(item, "resolver_policy_id") != Some(ALOS2_RESOLVER_POLICY_ID)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    let properties = match item.metadata.get("properties").and_then(Value::as_object) {
        Some(properties) => properties,
        None => {
            return invalid(ExecutionPolicyError::Structural(
                "properties missing".to_owned(),
            ))
        }
    };
    if properties
        .get("collection_short_name")
        .and_then(Value::as_str)
        != Some(ALOS2_COLLECTION_SHORT_NAME)
        || properties.get("collection_version").and_then(Value::as_str)
            != Some(ALOS2_COLLECTION_VERSION)
        || properties.get("platform").and_then(Value::as_str) != Some("ALOS-2")
        || properties.get("instrument").and_then(Value::as_str) != Some("PALSAR-2")
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    }
    let processing_level = properties.get("processing_level").and_then(Value::as_str);
    if metadata_text(item, "product_family") != Some(ALOS2_COLLECTION_SHORT_NAME) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductFamily);
    }
    if metadata_text(item, "processing_level") != processing_level {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }
    if !processing_level.is_some_and(|value| policy.approved_processing_levels.contains(&value)) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }
    let beam_mode = properties.get("beam_mode").and_then(Value::as_str);
    let polarizations = properties
        .get("polarizations")
        .and_then(Value::as_array)
        .and_then(|values| values.iter().map(Value::as_str).collect::<Option<Vec<_>>>());
    if !matches!(
        (beam_mode, polarizations.as_deref()),
        (Some(beam_mode), Some(polarizations))
            if alos2_expected_polarizations(beam_mode) == Some(polarizations)
    ) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    }

    let Some(access) = item.asset_access.as_ref() else {
        return invalid(ExecutionPolicyError::MissingAssetAccess);
    };
    if let Err(reason) = access.validate() {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(reason));
    }
    if !policy.approved_asset_roles.contains(&access.role) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AssetRole);
    }
    if access.descriptor.scheme != AssetAccessScheme::EarthdataHttps
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if !access.descriptor.requires_credentials
        || !access
            .descriptor
            .credential_profile
            .as_deref()
            .is_some_and(|value| policy.approved_credential_profiles.contains(&value))
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) || !file_name.to_ascii_lowercase().ends_with(".zip") {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0)
        || !valid_alos2_checksum(
            access.checksum_algorithm.as_deref(),
            access.checksum_value.as_deref(),
        )
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive size and valid checksum are required".to_owned(),
        ));
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "stable href is required".to_owned(),
        ));
    };
    if stable_href.rsplit('/').next() != Some(file_name) {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "stable href does not match the frozen filename".to_owned(),
        ));
    }

    let reference = &access.reference;
    let provider_product_id = metadata_text(item, "provider_product_id");
    let frozen_product_key = metadata_text(item, "provider_product_key");
    let Some(provider_product_id) =
        provider_product_id.filter(|value| valid_alos2_concept_id(value))
    else {
        return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id"));
    };
    let expected_product_key = match provider_product_key(
        ALOS2_PROVIDER_ID,
        ALOS2_NATIVE_COLLECTION_ID,
        provider_product_id,
    ) {
        Ok(value) => value,
        Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
    };
    if frozen_product_key != Some(expected_product_key.as_str())
        || reference.provider_id != ALOS2_PROVIDER_ID
        || reference.collection_id != ALOS2_NATIVE_COLLECTION_ID
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != expected_product_key
        || item.item_id != reference.provider_product_key
        || reference.canonical_product_key.is_some()
        || reference.asset_key != file_name
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key = match stable_asset_key(&reference.provider_product_key, file_name) {
        Ok(value) => value,
        Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
    };
    if reference.stable_asset_key != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    if !alos2_frozen_graph_matches(
        item,
        properties,
        access,
        provider_product_id,
        &expected_product_key,
        file_name,
    ) {
        return invalid(ExecutionPolicyError::IdentityMismatch("metadata_graph"));
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn valid_alos2_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_alos2_checksum(algorithm: Option<&str>, value: Option<&str>) -> bool {
    let (Some(algorithm), Some(value)) = (algorithm, value) else {
        return false;
    };
    let expected_length = match algorithm {
        "MD5" => 32,
        "SHA256" => 64,
        _ => return false,
    };
    value.len() == expected_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn alos2_frozen_graph_matches(
    item: &SnapshotItem,
    properties: &serde_json::Map<String, Value>,
    access: &FrozenAssetAccess,
    provider_product_id: &str,
    expected_product_key: &str,
    file_name: &str,
) -> bool {
    let reference = &access.reference;
    let property_identity_matches = properties
        .get("provider_product_key")
        .and_then(Value::as_str)
        == Some(expected_product_key)
        && properties
            .get("provider_product_id")
            .and_then(Value::as_str)
            == Some(provider_product_id)
        && properties
            .get("cmr_granule_concept_id")
            .and_then(Value::as_str)
            == Some(provider_product_id)
        && properties.get("logical_dataset").and_then(Value::as_str) == Some(ALOS2_LOGICAL_DATASET)
        && properties
            .get("collection_concept_id")
            .and_then(Value::as_str)
            == Some(ALOS2_NATIVE_COLLECTION_ID)
        && properties
            .get("collection_profile_id")
            .and_then(Value::as_str)
            == Some(ALOS2_PROFILE_ID)
        && properties.get("identity_rule_id").and_then(Value::as_str)
            == Some(ALOS2_IDENTITY_RULE_ID)
        && properties.get("filename_rule_id").and_then(Value::as_str)
            == Some(ALOS2_FILENAME_RULE_ID)
        && properties.get("resolver_policy_id").and_then(Value::as_str)
            == Some(ALOS2_RESOLVER_POLICY_ID);
    if !property_identity_matches {
        return false;
    }

    let Some(integrity) = properties
        .get("asset_integrity")
        .and_then(Value::as_object)
        .filter(|values| values.len() == 1)
        .and_then(|values| values.get(&reference.asset_key))
        .and_then(Value::as_object)
        .filter(|value| value.len() == 4)
    else {
        return false;
    };
    if integrity.get("file_name").and_then(Value::as_str) != Some(file_name)
        || integrity.get("size_bytes").and_then(Value::as_u64) != access.expected_size_bytes
        || integrity.get("algorithm").and_then(Value::as_str)
            != access.checksum_algorithm.as_deref()
        || integrity.get("value").and_then(Value::as_str) != access.checksum_value.as_deref()
    {
        return false;
    }

    let Some(assets) = item.metadata.get("assets").and_then(Value::as_array) else {
        return false;
    };
    let [asset] = assets.as_slice() else {
        return false;
    };
    let descriptor = serde_json::to_value(&access.descriptor).ok();
    if asset.get("asset_key").and_then(Value::as_str) != Some(reference.asset_key.as_str())
        || asset.get("stable_asset_key").and_then(Value::as_str)
            != Some(reference.stable_asset_key.as_str())
        || asset.get("access_descriptor") != descriptor.as_ref()
        || asset.get("role").and_then(Value::as_str) != Some("primary")
        || asset
            .get("native_roles")
            .and_then(Value::as_array)
            .is_none_or(|roles| {
                roles.len() != 1 || roles.first().and_then(Value::as_str) != Some("archive")
            })
        || asset.get("url").is_some_and(|value| !value.is_null())
        || asset.get("file_name").and_then(Value::as_str) != Some(file_name)
        || asset.get("media_type").and_then(Value::as_str) != Some("application/zip")
        || asset.get("size_bytes").and_then(Value::as_u64) != access.expected_size_bytes
    {
        return false;
    }

    let Some(routes) = properties
        .get("asset_routes")
        .and_then(Value::as_object)
        .filter(|values| values.len() == 1)
        .and_then(|values| values.get(&reference.asset_key))
        .and_then(Value::as_array)
        .filter(|routes| !routes.is_empty() && routes.len() <= 8)
    else {
        return false;
    };
    let mut https_count = 0usize;
    let mut seen = std::collections::BTreeSet::new();
    for route in routes {
        let Some(route) = route.as_object().filter(|value| value.len() == 2) else {
            return false;
        };
        let (Some(scheme), Some(locator)) = (
            route.get("scheme").and_then(Value::as_str),
            route.get("locator").and_then(Value::as_str),
        ) else {
            return false;
        };
        if !valid_alos2_route(scheme, locator, file_name) || !seen.insert((scheme, locator)) {
            return false;
        }
        if scheme == "https" {
            https_count += 1;
            if access.descriptor.stable_href.as_deref() != Some(locator) {
                return false;
            }
        }
    }
    https_count == 1
}

fn valid_alos2_route(scheme: &str, locator: &str, file_name: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(locator) else {
        return false;
    };
    let common_safe = parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed
            .path_segments()
            .and_then(Iterator::last)
            .is_some_and(|value| value == file_name);
    common_safe
        && match scheme {
            "https" => parsed.scheme() == "https" && parsed.port_or_known_default() == Some(443),
            "s3" => parsed.scheme() == "s3" && parsed.port().is_none(),
            _ => false,
        }
}
