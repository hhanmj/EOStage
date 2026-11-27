use super::{
    invalid, metadata_text, AssetExecutionPolicyDefinition, AuthRefreshPolicyId,
    EligibleResolverAsset, ExecutionDeliveryModel, ExecutionPolicyError,
    ResolverCutoverEligibility,
};
use crate::{
    ArtifactValidatorRegistry, NisarSsarRslcValidationContext, BHOONIDHI_CREDENTIAL_PROFILE,
    NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING,
};
use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessScheme, AssetRole, SnapshotItem,
};
use insar_provider::bhoonidhi::{
    BHOONIDHI_COLLECTION_ID, BHOONIDHI_FILENAME_RULE_ID, BHOONIDHI_IDENTITY_RULE_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_PROFILE_ID, BHOONIDHI_PROVIDER_ID,
    BHOONIDHI_RESOLVER_POLICY_ID,
};
use insar_provider::catalog::CollectionProfileRegistry;
use serde_json::Value;

pub(super) static BHOONIDHI_SSAR_COMPLETE_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: BHOONIDHI_RESOLVER_POLICY_ID,
        collection_profile_id: BHOONIDHI_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::NisarSsarArchiveFirst,
        approved_product_families: &[BHOONIDHI_COLLECTION_ID],
        approved_processing_levels: &["RSLC"],
        approved_resolver_ids: &[BHOONIDHI_RESOLVER_POLICY_ID],
        approved_credential_profiles: &[BHOONIDHI_CREDENTIAL_PROFILE],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::BhoonidhiOAuthBearerV1,
        artifact_validator_binding: Some(NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING),
    };

pub(super) fn evaluate_bhoonidhi_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
    validators: &ArtifactValidatorRegistry,
) -> ResolverCutoverEligibility {
    let fail = || {
        invalid(ExecutionPolicyError::IdentityMismatch(
            "bhoonidhi_frozen_identity",
        ))
    };
    let Some(profile) = CollectionProfileRegistry.get(BHOONIDHI_PROFILE_ID) else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    if policy.policy_id != BHOONIDHI_RESOLVER_POLICY_ID
        || policy.collection_profile_id != BHOONIDHI_PROFILE_ID
        || policy.artifact_validator_binding() != Some(NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING)
        || profile.provider_id != BHOONIDHI_PROVIDER_ID
        || profile.native_collection_id != BHOONIDHI_COLLECTION_ID
        || profile.resolver_policy_id != BHOONIDHI_RESOLVER_POLICY_ID
        || item.provider_id != BHOONIDHI_PROVIDER_ID
    {
        return fail();
    }
    for (key, expected) in [
        ("dataset", BHOONIDHI_LOGICAL_DATASET),
        ("logical_dataset", BHOONIDHI_LOGICAL_DATASET),
        ("native_collection_id", BHOONIDHI_COLLECTION_ID),
        ("collection_profile_id", BHOONIDHI_PROFILE_ID),
        ("identity_rule_id", BHOONIDHI_IDENTITY_RULE_ID),
        ("filename_rule_id", BHOONIDHI_FILENAME_RULE_ID),
        ("resolver_policy_id", BHOONIDHI_RESOLVER_POLICY_ID),
        ("delivery_model", "archive_first"),
        ("product_family", BHOONIDHI_COLLECTION_ID),
        ("processing_level", "RSLC"),
    ] {
        if metadata_text(item, key) != Some(expected) {
            return fail();
        }
    }
    let Some(id) = metadata_text(item, "provider_product_id") else {
        return fail();
    };
    let Ok(key) = provider_product_key(BHOONIDHI_PROVIDER_ID, BHOONIDHI_COLLECTION_ID, id) else {
        return fail();
    };
    let Some(access) = item.asset_access.as_ref() else {
        return invalid(ExecutionPolicyError::MissingAssetAccess);
    };
    let reference = &access.reference;
    if access.validate().is_err()
        || item.item_id != key
        || metadata_text(item, "provider_product_key") != Some(key.as_str())
        || reference.provider_id != BHOONIDHI_PROVIDER_ID
        || reference.collection_id != BHOONIDHI_COLLECTION_ID
        || reference.provider_product_id != id
        || reference.provider_product_key != key
        || reference.canonical_product_key.is_some()
        || reference.asset_key != "data"
        || stable_asset_key(&key, "data").ok().as_ref() != Some(&reference.stable_asset_key)
        || access.role != AssetRole::Primary
        || access.file_name.as_deref() != Some(format!("{id}.h5").as_str())
        || access.expected_size_bytes == Some(0)
        || access.checksum_algorithm.is_some()
        || access.checksum_value.is_some()
        || access.descriptor.resolver_id != BHOONIDHI_RESOLVER_POLICY_ID
        || access.descriptor.scheme != AssetAccessScheme::OauthBearerHttps
        || !access.descriptor.requires_credentials
        || access.descriptor.credential_profile.as_deref() != Some(BHOONIDHI_CREDENTIAL_PROFILE)
        || access.descriptor.stable_href.as_deref()
            != Some("https://bhoonidhi-api.nrsc.gov.in/download")
    {
        return fail();
    }
    let Some(properties) = item.metadata.get("properties").and_then(Value::as_object) else {
        return fail();
    };
    let text = |name: &str| properties.get(name).and_then(Value::as_str);
    for (name, expected) in [
        ("ID", id),
        ("Online", "Y"),
        ("Platform", "NISAR"),
        ("Instrument", "SSAR"),
        ("Producer", "ISRO"),
        ("ProductCode", "ST0000KED"),
        ("Mode", "W"),
        ("SoftwareVersion", "1.4.00"),
        ("mode", "W"),
        ("software_version", "1.4.00"),
    ] {
        if text(name) != Some(expected) {
            return fail();
        }
    }
    let Some(provider_polarization) = text("Polarization") else {
        return fail();
    };
    if text("polarization") != Some(provider_polarization) {
        return fail();
    }
    let partial = match text("Coverage") {
        Some("P") => true,
        Some("F") => false,
        _ => return fail(),
    };
    let context = match NisarSsarRslcValidationContext::from_frozen_bhoonidhi_deferred_size(
        id,
        format!("{id}.h5"),
        "1.4.00",
        "W",
        provider_polarization,
        partial,
    ) {
        Ok(context) => context,
        Err(_) => return fail(),
    };
    let plan = match validators.resolve(NISAR_SSAR_RSLC_STRUCTURE_VALIDATOR_BINDING, context.into())
    {
        Ok(plan) => plan,
        Err(_) => return invalid(ExecutionPolicyError::ArtifactValidationUnavailable),
    };
    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: Some(plan),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::{
        AssetAccessDescriptor, AssetExecutionAuthority, AssetExecutionPolicy,
        AssetResolverParameters, FrozenAssetAccess, StableAssetReference,
    };
    use serde_json::{json, Map, Value};

    const DHNA_PRODUCT_ID: &str =
        "NISAR_S1_PR_RSLC_030_026_A_020_3700_DHNA_A_20260907T234951_20260907T235005_P00500_M_P_I_001";

    fn dhna_item(provider_polarization: &str) -> SnapshotItem {
        let item_id = provider_product_key(
            BHOONIDHI_PROVIDER_ID,
            BHOONIDHI_COLLECTION_ID,
            DHNA_PRODUCT_ID,
        )
        .expect("reviewed provider product key");
        let stable_key = stable_asset_key(&item_id, "data").expect("stable data key");
        let file_name = format!("{DHNA_PRODUCT_ID}.h5");
        let properties = json!({
            "ID": DHNA_PRODUCT_ID,
            "Online": "Y",
            "Platform": "NISAR",
            "Instrument": "SSAR",
            "Producer": "ISRO",
            "ProductCode": "ST0000KED",
            "Mode": "W",
            "Polarization": provider_polarization,
            "SoftwareVersion": "1.4.00",
            "Coverage": "P",
            "mode": "W",
            "polarization": provider_polarization,
            "software_version": "1.4.00"
        });
        SnapshotItem {
            item_id: item_id.clone(),
            provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
            display_name: DHNA_PRODUCT_ID.to_owned(),
            acquisition_datetime: Some("2026-09-07T23:49:51Z".to_owned()),
            footprint_geojson: Some(
                json!({
                    "type": "Polygon",
                    "coordinates": [[[75.0, 5.0], [76.0, 5.0], [76.0, 6.0], [75.0, 6.0], [75.0, 5.0]]]
                })
                .as_object()
                .expect("polygon object")
                .clone(),
            ),
            metadata: Map::from_iter([
                (
                    "dataset".to_owned(),
                    Value::String(BHOONIDHI_LOGICAL_DATASET.to_owned()),
                ),
                (
                    "logical_dataset".to_owned(),
                    Value::String(BHOONIDHI_LOGICAL_DATASET.to_owned()),
                ),
                (
                    "native_collection_id".to_owned(),
                    Value::String(BHOONIDHI_COLLECTION_ID.to_owned()),
                ),
                (
                    "collection_profile_id".to_owned(),
                    Value::String(BHOONIDHI_PROFILE_ID.to_owned()),
                ),
                (
                    "identity_rule_id".to_owned(),
                    Value::String(BHOONIDHI_IDENTITY_RULE_ID.to_owned()),
                ),
                (
                    "filename_rule_id".to_owned(),
                    Value::String(BHOONIDHI_FILENAME_RULE_ID.to_owned()),
                ),
                (
                    "resolver_policy_id".to_owned(),
                    Value::String(BHOONIDHI_RESOLVER_POLICY_ID.to_owned()),
                ),
                (
                    "delivery_model".to_owned(),
                    Value::String("archive_first".to_owned()),
                ),
                (
                    "product_family".to_owned(),
                    Value::String(BHOONIDHI_COLLECTION_ID.to_owned()),
                ),
                (
                    "processing_level".to_owned(),
                    Value::String("RSLC".to_owned()),
                ),
                (
                    "provider_product_id".to_owned(),
                    Value::String(DHNA_PRODUCT_ID.to_owned()),
                ),
                (
                    "provider_product_key".to_owned(),
                    Value::String(item_id.clone()),
                ),
                ("properties".to_owned(), properties),
            ]),
            payload: Map::new(),
            asset_access: Some(FrozenAssetAccess {
                reference: StableAssetReference {
                    provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
                    collection_id: BHOONIDHI_COLLECTION_ID.to_owned(),
                    provider_product_id: DHNA_PRODUCT_ID.to_owned(),
                    provider_product_key: item_id,
                    canonical_product_key: None,
                    asset_key: "data".to_owned(),
                    stable_asset_key: stable_key,
                },
                descriptor: AssetAccessDescriptor {
                    resolver_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                    scheme: AssetAccessScheme::OauthBearerHttps,
                    stable_href: Some("https://bhoonidhi-api.nrsc.gov.in/download".to_owned()),
                    requires_credentials: true,
                    credential_profile: Some(BHOONIDHI_CREDENTIAL_PROFILE.to_owned()),
                    resolver_parameters: AssetResolverParameters {},
                },
                role: AssetRole::Primary,
                file_name: Some(file_name),
                expected_size_bytes: None,
                checksum_algorithm: None,
                checksum_value: None,
            }),
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: BHOONIDHI_RESOLVER_POLICY_ID.to_owned(),
                collection_profile_id: Some(BHOONIDHI_PROFILE_ID.to_owned()),
            }),
        }
    }

    #[test]
    fn dhna_complete_product_policy_requires_exact_provider_polarization() {
        let validators = ArtifactValidatorRegistry::standard();
        let item = dhna_item("DH");
        assert!(matches!(
            evaluate_bhoonidhi_resolver_cutover_eligibility(
                &item,
                &BHOONIDHI_SSAR_COMPLETE_POLICY,
                &validators,
            ),
            ResolverCutoverEligibility::Eligible(_)
        ));

        for mismatched_polarization in ["CP", "DP", "SH", "DV", "UNKNOWN"] {
            let item = dhna_item(mismatched_polarization);
            assert!(
                matches!(
                    evaluate_bhoonidhi_resolver_cutover_eligibility(
                        &item,
                        &BHOONIDHI_SSAR_COMPLETE_POLICY,
                        &validators,
                    ),
                    ResolverCutoverEligibility::Invalid(_)
                ),
                "DHNA must reject {mismatched_polarization}"
            );
        }

        let mut mismatched = dhna_item("DH");
        mismatched
            .metadata
            .get_mut("properties")
            .and_then(Value::as_object_mut)
            .expect("properties")
            .insert("polarization".to_owned(), Value::String("CP".to_owned()));
        assert!(matches!(
            evaluate_bhoonidhi_resolver_cutover_eligibility(
                &mismatched,
                &BHOONIDHI_SSAR_COMPLETE_POLICY,
                &validators,
            ),
            ResolverCutoverEligibility::Invalid(_)
        ));

        let unknown = dhna_item("UNKNOWN");
        assert!(matches!(
            evaluate_bhoonidhi_resolver_cutover_eligibility(
                &unknown,
                &BHOONIDHI_SSAR_COMPLETE_POLICY,
                &validators,
            ),
            ResolverCutoverEligibility::Invalid(_)
        ));
    }
}
