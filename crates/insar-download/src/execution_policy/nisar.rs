use std::collections::BTreeSet;

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessScheme, AssetRole, FrozenAssetAccess,
    SnapshotItem,
};
use insar_provider::asf::nisar::{
    NISAR_MAX_ASSET_GRAPH_JSON_BYTES, NISAR_MAX_DEDUPLICATED_ASSETS, NISAR_MAX_FILE_NAME_BYTES,
    NISAR_MAX_LOCATOR_BYTES, NISAR_MAX_MEDIA_TYPE_BYTES, NISAR_MAX_NATIVE_ROLES,
    NISAR_MAX_NATIVE_ROLE_BYTES, NISAR_MAX_RAW_RESOURCE_ENTRIES,
};
use insar_provider::catalog::{
    CollectionProfileRegistry, NisarRslcProductSpecificationProfileRegistry, NISAR_BAND,
    NISAR_COLLECTION_SHORT_NAME, NISAR_COLLECTION_VERSION, NISAR_FILENAME_RULE_ID,
    NISAR_FLIGHT_DIRECTIONS, NISAR_FRAME_COVERAGES, NISAR_FREQUENCY_A_POLARIZATIONS,
    NISAR_FREQUENCY_B_POLARIZATIONS, NISAR_IDENTITY_RULE_ID, NISAR_LOGICAL_DATASET,
    NISAR_LOOK_DIRECTIONS, NISAR_NATIVE_COLLECTION_ID, NISAR_PRODUCTION_CONFIGURATION,
    NISAR_PROFILE_ID, NISAR_PROVIDER_ID, NISAR_QUALITY_STATUS, NISAR_RANGE_BANDWIDTHS,
    NISAR_REQUIRED_PRODUCT_TYPE, NISAR_RESOLVER_POLICY_ID, NISAR_RESOLVER_POLICY_ID_V1,
    NISAR_RESOLVER_POLICY_ID_V2, NISAR_SCIENCE_LEVEL,
};
use reqwest::Url;
use serde_json::{Map, Value};

use super::{
    invalid, metadata_text, safe_basename, AssetExecutionPolicyDefinition, AuthRefreshPolicyId,
    EligibilityReason, EligibleResolverAsset, ExecutionDeliveryModel, ExecutionPolicyError,
    ResolverCutoverEligibility,
};
use crate::execution_policy_registry::exact_nisar_primary_href;
use crate::{
    ArtifactValidationContext, ArtifactValidatorRegistry, NisarRslcValidationContext,
    NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING,
};

pub(super) static ASF_NISAR_COMPLETE_HDF5_POLICY_V1: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: NISAR_RESOLVER_POLICY_ID_V1,
        collection_profile_id: NISAR_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::NisarArchiveFirst,
        approved_product_families: &[NISAR_COLLECTION_SHORT_NAME],
        approved_processing_levels: &[NISAR_REQUIRED_PRODUCT_TYPE],
        approved_resolver_ids: &["earthdata_https"],
        approved_credential_profiles: &["earthdata-default"],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::EarthdataStandardV1,
        artifact_validator_binding: Some(NISAR_RSLC_STRUCTURE_VALIDATOR_BINDING),
    };

pub(super) static ASF_NISAR_COMPLETE_HDF5_POLICY_V2: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: NISAR_RESOLVER_POLICY_ID_V2,
        collection_profile_id: NISAR_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::NisarArchiveFirst,
        approved_product_families: &[NISAR_COLLECTION_SHORT_NAME],
        approved_processing_levels: &[NISAR_REQUIRED_PRODUCT_TYPE],
        approved_resolver_ids: &["earthdata_https"],
        approved_credential_profiles: &["earthdata-default"],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::EarthdataStandardV1,
        artifact_validator_binding: Some(NISAR_RSLC_STRUCTURE_VALIDATOR_V2_BINDING),
    };

pub(super) fn evaluate_nisar_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
    artifact_validators: &ArtifactValidatorRegistry,
) -> ResolverCutoverEligibility {
    let Some(profile) = CollectionProfileRegistry.get(NISAR_PROFILE_ID) else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    let is_v1 = policy.policy_id == NISAR_RESOLVER_POLICY_ID_V1;
    let is_v2 = policy.policy_id == NISAR_RESOLVER_POLICY_ID_V2;
    if (!is_v1 && !is_v2)
        || policy.collection_profile_id != NISAR_PROFILE_ID
        || policy.delivery_model != ExecutionDeliveryModel::NisarArchiveFirst
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::EarthdataStandardV1
        || profile.provider_id != NISAR_PROVIDER_ID
        || profile.native_collection_id != NISAR_NATIVE_COLLECTION_ID
        || profile.resolver_policy_id != NISAR_RESOLVER_POLICY_ID
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != NISAR_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "dataset") != Some(NISAR_LOGICAL_DATASET)
        || metadata_text(item, "logical_dataset") != Some(NISAR_LOGICAL_DATASET)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(NISAR_NATIVE_COLLECTION_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(NISAR_PROFILE_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(NISAR_IDENTITY_RULE_ID)
        || metadata_text(item, "filename_rule_id") != Some(NISAR_FILENAME_RULE_ID)
        || metadata_text(item, "resolver_policy_id") != Some(policy.policy_id)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    if metadata_text(item, "product_family") != Some(NISAR_COLLECTION_SHORT_NAME) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductFamily);
    }
    if metadata_text(item, "processing_level") != Some(NISAR_REQUIRED_PRODUCT_TYPE) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }
    if metadata_text(item, "quality_status") != Some(NISAR_QUALITY_STATUS) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    }

    let properties = match item.metadata.get("properties").and_then(Value::as_object) {
        Some(properties) => properties,
        None => {
            return invalid(ExecutionPolicyError::Structural(
                "NISAR properties missing".to_owned(),
            ))
        }
    };
    if !valid_nisar_scope(item, properties, policy.policy_id) {
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
    if !safe_basename(file_name)
        || file_name != format!("{}.h5", item.display_name)
        || file_name.len() > NISAR_MAX_FILE_NAME_BYTES
    {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0)
        || !valid_nisar_checksum(
            access.checksum_algorithm.as_deref(),
            access.checksum_value.as_deref(),
            false,
        )
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive size and canonical checksum are required".to_owned(),
        ));
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "stable href is required".to_owned(),
        ));
    };
    if !exact_nisar_primary_href(stable_href, Some(file_name)) {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "stable href is not an official NISAR product route".to_owned(),
        ));
    }

    let reference = &access.reference;
    let Some(provider_product_id) =
        metadata_text(item, "provider_product_id").filter(|value| valid_nisar_concept_id(value))
    else {
        return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id"));
    };
    let expected_product_key = match provider_product_key(
        NISAR_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID,
        provider_product_id,
    ) {
        Ok(value) => value,
        Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
    };
    if metadata_text(item, "provider_product_key") != Some(expected_product_key.as_str())
        || reference.provider_id != NISAR_PROVIDER_ID
        || reference.collection_id != NISAR_NATIVE_COLLECTION_ID
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != expected_product_key
        || item.item_id != reference.provider_product_key
        || reference.canonical_product_key.is_some()
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, &reference.asset_key) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    if reference.stable_asset_key != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    if !nisar_frozen_graph_matches(
        item,
        properties,
        access,
        provider_product_id,
        &expected_product_key,
        file_name,
    ) {
        return invalid(ExecutionPolicyError::IdentityMismatch("metadata_graph"));
    }

    let Some(binding) = policy.artifact_validator_binding() else {
        return invalid(ExecutionPolicyError::ArtifactValidationUnavailable);
    };
    let frequency_a = properties
        .get("frequency_a_polarization")
        .and_then(Value::as_str);
    let frequency_b = properties
        .get("frequency_b_polarization")
        .and_then(Value::as_str);
    let context = match if is_v2 {
        NisarRslcValidationContext::from_frozen_product_profile(
            item.display_name.clone(),
            frequency_a,
            frequency_b,
            properties.get("crid").and_then(Value::as_str).unwrap_or(""),
            properties
                .get("product_specification_version")
                .and_then(Value::as_str)
                .unwrap_or(""),
            properties
                .get("product_specification_profile_registry_version")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0),
        )
    } else {
        NisarRslcValidationContext::from_frozen_metadata(
            item.display_name.clone(),
            frequency_a,
            frequency_b,
        )
    } {
        Ok(context) => ArtifactValidationContext::from(context),
        Err(_) => return invalid(ExecutionPolicyError::ArtifactValidationUnavailable),
    };
    let artifact_validation_plan = match artifact_validators.resolve(binding, context) {
        Ok(plan) => plan,
        Err(_) => return invalid(ExecutionPolicyError::ArtifactValidationUnavailable),
    };

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: Some(artifact_validation_plan),
    }))
}

fn valid_nisar_scope(
    item: &SnapshotItem,
    properties: &Map<String, Value>,
    policy_id: &str,
) -> bool {
    let property_text = |key| properties.get(key).and_then(Value::as_str);
    let identity_matches = property_text("provider_product_key") == Some(item.item_id.as_str())
        && property_text("provider_product_id") == metadata_text(item, "provider_product_id")
        && property_text("cmr_granule_concept_id") == metadata_text(item, "provider_product_id")
        && property_text("producer_granule_id") == Some(item.display_name.as_str())
        && property_text("logical_dataset") == Some(NISAR_LOGICAL_DATASET)
        && property_text("collection_concept_id") == Some(NISAR_NATIVE_COLLECTION_ID)
        && property_text("collection_short_name") == Some(NISAR_COLLECTION_SHORT_NAME)
        && property_text("collection_version") == Some(NISAR_COLLECTION_VERSION)
        && property_text("collection_profile_id") == Some(NISAR_PROFILE_ID)
        && property_text("identity_rule_id") == Some(NISAR_IDENTITY_RULE_ID)
        && property_text("filename_rule_id") == Some(NISAR_FILENAME_RULE_ID)
        && property_text("resolver_policy_id") == Some(policy_id);
    let scope_matches = property_text("processing_level") == Some(NISAR_REQUIRED_PRODUCT_TYPE)
        && property_text("science_level") == Some(NISAR_SCIENCE_LEVEL)
        && property_text("quality_status") == Some(NISAR_QUALITY_STATUS)
        && property_text("production_configuration") == Some(NISAR_PRODUCTION_CONFIGURATION)
        && property_text("band") == Some(NISAR_BAND)
        && property_text("frame_coverage")
            .is_some_and(|value| NISAR_FRAME_COVERAGES.contains(&value))
        && property_text("flight_direction")
            .is_some_and(|value| NISAR_FLIGHT_DIRECTIONS.contains(&value))
        && property_text("look_direction")
            .is_some_and(|value| NISAR_LOOK_DIRECTIONS.contains(&value))
        && property_text("range_bandwidth")
            .is_some_and(|value| NISAR_RANGE_BANDWIDTHS.contains(&value))
        && properties
            .get("joint_observation")
            .is_some_and(Value::is_boolean)
        && properties.get("full_product").and_then(Value::as_bool) == Some(true);
    let frame = properties.get("frame").and_then(Value::as_u64);
    let track = properties.get("track").and_then(Value::as_u64);
    let relative_orbit = properties.get("relative_orbit").and_then(Value::as_u64);
    let orbit_matches = frame.is_some_and(|value| value > 0)
        && track.is_some_and(|value| value > 0)
        && relative_orbit == track;
    let frequency_a = property_text("frequency_a_polarization");
    let frequency_b = property_text("frequency_b_polarization");
    let frequencies_match = (frequency_a.is_some() || frequency_b.is_some())
        && (!properties.contains_key("frequency_a_polarization") || frequency_a.is_some())
        && (!properties.contains_key("frequency_b_polarization") || frequency_b.is_some())
        && frequency_a.is_none_or(|value| NISAR_FREQUENCY_A_POLARIZATIONS.contains(&value))
        && frequency_b.is_none_or(|value| NISAR_FREQUENCY_B_POLARIZATIONS.contains(&value));
    let product_specification_profile_matches = policy_id == NISAR_RESOLVER_POLICY_ID_V1
        || (policy_id == NISAR_RESOLVER_POLICY_ID_V2
            && property_text("crid")
                .and_then(|crid| NisarRslcProductSpecificationProfileRegistry.for_crid(crid))
                .is_some_and(|profile| {
                    property_text("product_specification_version")
                        == Some(profile.product_specification_version)
                        && properties
                            .get("product_specification_profile_registry_version")
                            .and_then(Value::as_u64)
                            == Some(u64::from(profile.registry_version))
                        && profile.resolver_policy_id == policy_id
                }));
    identity_matches
        && scope_matches
        && orbit_matches
        && frequencies_match
        && product_specification_profile_matches
}

fn nisar_frozen_graph_matches(
    item: &SnapshotItem,
    properties: &Map<String, Value>,
    access: &FrozenAssetAccess,
    provider_product_id: &str,
    expected_product_key: &str,
    expected_file_name: &str,
) -> bool {
    if properties.get("primary_asset_key").and_then(Value::as_str)
        != Some(access.reference.asset_key.as_str())
        || properties
            .get("provider_product_key")
            .and_then(Value::as_str)
            != Some(expected_product_key)
        || properties
            .get("provider_product_id")
            .and_then(Value::as_str)
            != Some(provider_product_id)
    {
        return false;
    }
    let Some(assets) = item.metadata.get("assets").and_then(Value::as_array) else {
        return false;
    };
    if assets.is_empty()
        || assets.len() > NISAR_MAX_DEDUPLICATED_ASSETS
        || serde_json::to_vec(assets)
            .map_or(true, |value| value.len() > NISAR_MAX_ASSET_GRAPH_JSON_BYTES)
    {
        return false;
    }

    let mut asset_keys = BTreeSet::new();
    let mut stable_keys = BTreeSet::new();
    let mut previous_sort_key: Option<(u8, String, String, String)> = None;
    let mut represented_resources = 0usize;
    let mut primary_count = 0usize;
    let mut preview_count = 0usize;
    let mut browse_count = 0usize;
    let mut thumbnail_count = 0usize;
    let mut primary_matches = false;
    for value in assets {
        let Some(asset) = value.as_object().filter(|asset| {
            exact_keys(
                asset,
                &[
                    "asset_key",
                    "stable_asset_key",
                    "role",
                    "native_roles",
                    "file_name",
                    "media_type",
                    "size_bytes",
                    "checksum",
                    "routes",
                    "native_evidence",
                ],
            )
        }) else {
            return false;
        };
        let (Some(asset_key), Some(stable_key), Some(role), Some(file_name), Some(media_type)) = (
            bounded_text(asset.get("asset_key"), NISAR_MAX_FILE_NAME_BYTES),
            bounded_text(asset.get("stable_asset_key"), 2_048),
            asset.get("role").and_then(Value::as_str),
            bounded_text(asset.get("file_name"), NISAR_MAX_FILE_NAME_BYTES),
            bounded_text(asset.get("media_type"), NISAR_MAX_MEDIA_TYPE_BYTES),
        ) else {
            return false;
        };
        let Some(role_rank) = role_rank(role) else {
            return false;
        };
        if !safe_basename(file_name)
            || !valid_media_type(media_type)
            || forbidden_scope_text(asset_key)
            || forbidden_scope_text(file_name)
            || forbidden_scope_text(media_type)
            || !asset_keys.insert(asset_key.to_owned())
            || !stable_keys.insert(stable_key.to_owned())
            || stable_asset_key(expected_product_key, asset_key)
                .ok()
                .as_deref()
                != Some(stable_key)
        {
            return false;
        }
        let sort_key = (
            role_rank,
            file_name.to_owned(),
            media_type.to_owned(),
            stable_key.to_owned(),
        );
        if previous_sort_key
            .as_ref()
            .is_some_and(|previous| previous > &sort_key)
        {
            return false;
        }
        previous_sort_key = Some(sort_key);

        let Some(native_roles) = string_array(asset.get("native_roles")) else {
            return false;
        };
        let Some(evidence) = asset.get("native_evidence").and_then(Value::as_object) else {
            return false;
        };
        let Some(parsed_evidence) = parse_native_evidence(evidence, file_name, &native_roles)
        else {
            return false;
        };
        represented_resources += usize::from(parsed_evidence.archive_name.is_some());

        let size = match asset.get("size_bytes") {
            Some(Value::Null) => None,
            Some(value) => value.as_u64().filter(|value| *value > 0),
            None => return false,
        };
        if !asset.get("size_bytes").is_some_and(Value::is_null) && size.is_none() {
            return false;
        }
        let checksum = match asset.get("checksum") {
            Some(Value::Null) => None,
            Some(value) => match parse_checksum(value) {
                Some(value) => Some(value),
                None => return false,
            },
            None => return false,
        };
        if size.is_some() != checksum.is_some() {
            return false;
        }
        let Some(routes) = asset.get("routes").and_then(Value::as_array) else {
            return false;
        };
        represented_resources += routes.len();
        let mut seen_routes = BTreeSet::new();
        let mut https_routes = Vec::new();
        for route in routes {
            let Some(route) = route
                .as_object()
                .filter(|value| exact_keys(value, &["scheme", "locator"]))
            else {
                return false;
            };
            let (Some(scheme), Some(locator)) = (
                route.get("scheme").and_then(Value::as_str),
                bounded_text(route.get("locator"), NISAR_MAX_LOCATOR_BYTES),
            ) else {
                return false;
            };
            if !valid_nisar_route(scheme, locator, file_name)
                || !seen_routes.insert((scheme, locator))
            {
                return false;
            }
            if scheme == "https" {
                https_routes.push(locator);
            }
        }

        match role {
            "primary" => {
                primary_count += 1;
                if !valid_primary_evidence(&parsed_evidence, file_name)
                    || media_type != "application/x-hdf5"
                    || file_name != expected_file_name
                    || size != access.expected_size_bytes
                    || checksum.is_none_or(|(algorithm, value)| {
                        canonical_checksum_algorithm(algorithm)
                            != access.checksum_algorithm.as_deref()
                            || Some(value) != access.checksum_value.as_deref()
                    })
                    || https_routes.as_slice()
                        != [access.descriptor.stable_href.as_deref().unwrap_or_default()]
                {
                    return false;
                }
                primary_matches = asset_key == access.reference.asset_key
                    && stable_key == access.reference.stable_asset_key;
            }
            "preview" | "browse" => {
                if role == "preview" {
                    preview_count += 1;
                } else {
                    browse_count += 1;
                }
                if !valid_image_evidence(&parsed_evidence, file_name, false)
                    || !media_type.starts_with("image/")
                    || https_routes.len() != 1
                {
                    return false;
                }
            }
            "thumbnail" => {
                thumbnail_count += 1;
                if !valid_image_evidence(&parsed_evidence, file_name, true)
                    || !media_type.starts_with("image/")
                    || https_routes.len() != 1
                {
                    return false;
                }
            }
            "metadata" | "auxiliary" => {
                if parsed_evidence.product_match
                    || parsed_evidence
                        .url_types
                        .iter()
                        .any(|value| *value == "GET DATA" || *value == "GET DATA VIA DIRECT ACCESS")
                    || (file_name.to_ascii_lowercase().ends_with(".h5") && role != "auxiliary")
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    primary_count == 1
        && primary_matches
        && thumbnail_count <= 1
        && represented_resources <= NISAR_MAX_RAW_RESOURCE_ENTRIES
        && properties.get("preview_available").and_then(Value::as_bool)
            == Some(preview_count > 0 || browse_count > 0)
}

struct ParsedEvidence<'a> {
    archive_name: Option<&'a str>,
    product_match: bool,
    url_types: Vec<&'a str>,
    url_subtypes: Vec<&'a str>,
    formats: Vec<&'a str>,
}

fn parse_native_evidence<'a>(
    evidence: &'a Map<String, Value>,
    file_name: &str,
    native_roles: &[&'a str],
) -> Option<ParsedEvidence<'a>> {
    if !exact_keys(
        evidence,
        &[
            "archive_name",
            "product_match",
            "url_types",
            "url_subtypes",
            "formats",
        ],
    ) {
        return None;
    }
    let archive_name = match evidence.get("archive_name")? {
        Value::Null => None,
        value => Some(bounded_text(Some(value), NISAR_MAX_FILE_NAME_BYTES)?),
    };
    if archive_name.is_some_and(|value| value != file_name) {
        return None;
    }
    let product_match = evidence.get("product_match")?.as_bool()?;
    let url_types = string_array(evidence.get("url_types"))?;
    let url_subtypes = string_array(evidence.get("url_subtypes"))?;
    let formats = string_array(evidence.get("formats"))?;
    let mut allowed_roles = BTreeSet::new();
    allowed_roles.extend(url_types.iter().copied());
    allowed_roles.extend(url_subtypes.iter().copied());
    let archive_roles = formats
        .iter()
        .map(|format| format!("ARCHIVE:{format}"))
        .collect::<Vec<_>>();
    if archive_name.is_some() {
        allowed_roles.extend(archive_roles.iter().map(String::as_str));
    }
    if native_roles.is_empty()
        || native_roles
            .iter()
            .any(|role| !allowed_roles.contains(role))
        || (archive_name.is_some()
            && archive_roles
                .iter()
                .any(|role| !native_roles.contains(&role.as_str())))
        || (archive_name.is_none() && native_roles.iter().any(|role| role.starts_with("ARCHIVE:")))
    {
        return None;
    }
    let evidence_text = native_roles
        .iter()
        .chain(url_types.iter())
        .chain(url_subtypes.iter())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_uppercase();
    if ["S-BAND", "S_BAND", "BHOONIDHI", "PRIVATE"]
        .iter()
        .any(|value| evidence_text.contains(value))
    {
        return None;
    }
    Some(ParsedEvidence {
        archive_name,
        product_match,
        url_types,
        url_subtypes,
        formats,
    })
}

fn valid_primary_evidence(evidence: &ParsedEvidence<'_>, file_name: &str) -> bool {
    let data_roles = ["GET DATA", "GET DATA VIA DIRECT ACCESS"];
    evidence.archive_name == Some(file_name)
        && evidence.product_match
        && evidence.formats.as_slice() == ["HDF5"]
        && evidence.url_subtypes.is_empty()
        && !evidence.url_types.is_empty()
        && evidence
            .url_types
            .iter()
            .all(|value| data_roles.contains(value))
}

fn valid_image_evidence(evidence: &ParsedEvidence<'_>, file_name: &str, thumbnail: bool) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let image_format = (lower.ends_with(".png") && evidence.formats.as_slice() == ["PNG"])
        || ((lower.ends_with(".jpg") || lower.ends_with(".jpeg"))
            && matches!(evidence.formats.as_slice(), ["JPEG"] | ["JPG"]));
    !evidence.product_match
        && evidence.url_types.as_slice() == ["GET RELATED VISUALIZATION"]
        && evidence.url_subtypes.is_empty()
        && image_format
        && lower.contains("thumbnail") == thumbnail
}

fn parse_checksum(value: &Value) -> Option<(&str, &str)> {
    let checksum = value.as_object()?;
    if !exact_keys(checksum, &["algorithm", "value"]) {
        return None;
    }
    let algorithm = checksum.get("algorithm")?.as_str()?;
    let value = checksum.get("value")?.as_str()?;
    valid_nisar_checksum(Some(algorithm), Some(value), true).then_some((algorithm, value))
}

fn valid_nisar_checksum(
    algorithm: Option<&str>,
    value: Option<&str>,
    allow_source_sha_spelling: bool,
) -> bool {
    let (Some(algorithm), Some(value)) = (algorithm, value) else {
        return false;
    };
    let expected = match algorithm {
        "MD5" => 32,
        "SHA256" => 64,
        "SHA-256" if allow_source_sha_spelling => 64,
        _ => return false,
    };
    value.len() == expected
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn canonical_checksum_algorithm(value: &str) -> Option<&str> {
    match value {
        "MD5" => Some("MD5"),
        "SHA256" | "SHA-256" => Some("SHA256"),
        _ => None,
    }
}

fn valid_nisar_concept_id(value: &str) -> bool {
    value
        .strip_prefix('G')
        .and_then(|value| value.strip_suffix("-ASF"))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_nisar_route(scheme: &str, locator: &str, file_name: &str) -> bool {
    let Ok(parsed) = Url::parse(locator) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let common_safe = locator.len() <= NISAR_MAX_LOCATOR_BYTES
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !parsed.path().to_ascii_lowercase().contains("/private/")
        && parsed
            .path_segments()
            .and_then(Iterator::last)
            .is_some_and(|value| value == file_name);
    common_safe
        && match scheme {
            "https" => {
                parsed.scheme() == "https"
                    && host == "nisar.asf.earthdatacloud.nasa.gov"
                    && parsed.port_or_known_default() == Some(443)
            }
            "s3" => {
                parsed.scheme() == "s3"
                    && matches!(
                        host,
                        "sds-n-cumulus-prod-nisar-products" | "sds-n-cumulus-prod-nisar-browse"
                    )
                    && parsed.port().is_none()
            }
            _ => false,
        }
}

fn bounded_text(value: Option<&Value>, maximum: usize) -> Option<&str> {
    value.and_then(Value::as_str).filter(|value| {
        !value.is_empty()
            && *value == value.trim()
            && value.len() <= maximum
            && !value.chars().any(char::is_control)
    })
}

fn string_array(value: Option<&Value>) -> Option<Vec<&str>> {
    let values = value?.as_array()?;
    if values.len() > NISAR_MAX_NATIVE_ROLES {
        return None;
    }
    let parsed = values
        .iter()
        .map(|value| bounded_text(Some(value), NISAR_MAX_NATIVE_ROLE_BYTES))
        .collect::<Option<Vec<_>>>()?;
    (parsed.iter().collect::<BTreeSet<_>>().len() == parsed.len()).then_some(parsed)
}

fn valid_media_type(value: &str) -> bool {
    value.split_once('/').is_some_and(|(kind, subtype)| {
        !kind.is_empty()
            && !subtype.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b';')
    })
}

fn role_rank(role: &str) -> Option<u8> {
    match role {
        "primary" => Some(0),
        "preview" => Some(1),
        "browse" => Some(2),
        "thumbnail" => Some(3),
        "metadata" => Some(4),
        "auxiliary" => Some(5),
        _ => None,
    }
}

fn forbidden_scope_text(value: &str) -> bool {
    let value = value.to_ascii_uppercase();
    ["S-BAND", "S_BAND", "BHOONIDHI", "PRIVATE"]
        .iter()
        .any(|forbidden| value.contains(forbidden))
}

fn exact_keys(map: &Map<String, Value>, expected: &[&str]) -> bool {
    map.len() == expected.len() && expected.iter().all(|key| map.contains_key(*key))
}
