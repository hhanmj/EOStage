//! Closed catalogue policy validation for reviewed catalogue archive routes.
//!
//! It constructs immutable evidence only. Runtime credential lookup, resolver
//! selection, and HTTP transfer remain owned by `ResolverExecutionRuntime`.

use std::collections::HashSet;

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, SnapshotItem, StableAssetReference,
};
use insar_provider::catalog::{
    validate_earth_search_sentinel2_stable_band_url, CollectionProfileRegistry, ALOS2_PROFILE_ID,
    ALOS2_RESOLVER_POLICY_ID, BHOONIDHI_PROFILE_ID, BHOONIDHI_RESOLVER_POLICY_ID,
    CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID, CDSE_SENTINEL2_L2A_RESOLVER_POLICY_ID,
    CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID, EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID, EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    NISAR_COLLECTION_SHORT_NAME, NISAR_PROFILE_ID, NISAR_RESOLVER_POLICY_ID_V1,
    NISAR_RESOLVER_POLICY_ID_V2, SENTINEL2_L1C_PROFILE_ID, SENTINEL2_L2A_PROFILE_ID,
    SENTINEL3_STANDARD_PROFILE_ID, USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V1,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2,
    USGS_LANDSAT_C2_L2_LEGACY_SCENE_FILENAME_RULE_ID,
};
use insar_provider::cdse::archive_filename::{safe_cdse_odata_name, ArchiveFilenameEvidence};
use insar_provider::planetary_computer::{
    validate_planetary_computer_landsat_asset,
    validate_planetary_computer_landsat_manifest_asset_keys, PlanetaryComputerLandsatAsset,
    PlanetaryComputerLandsatManifest,
};
use insar_provider::planetary_computer_asset::{
    validate_planetary_computer_reviewed_asset_identity, PlanetaryComputerReviewedAssetClass,
    PlanetaryComputerReviewedAssetEvidence, MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
    PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use reqwest::Url;
use serde_json::{Map, Value};

use insar_provider::usgs_m2m::{
    is_supported_landsat_c2_l2_bundle_option, UsgsM2mFullBundleOption,
    LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME, USGS_M2M_SESSION_PROFILE_ID,
};

use crate::asset_resolver::cdse::{
    canonical_cdse_odata_uuid, exact_cdse_archive_href, safe_cdse_archive_basename,
    stable_cdse_archive_href, valid_cdse_checksum, CDSE_ARCHIVE_ASSET_KEY,
    CDSE_ARCHIVE_RESOLVER_ID, CDSE_CREDENTIAL_PROFILE, CDSE_PROVIDER_ID,
};
use crate::asset_resolver::usgs_m2m::{
    parse_frozen_landsat_bundle_locator_v1, parse_frozen_landsat_bundle_locator_v2,
};
pub use crate::asset_resolver::usgs_m2m::{
    USGS_LANDSAT_C2_L2_RESOLVER_ID, USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
    USGS_LANDSAT_C2_L2_RESOLVER_ID_V2,
};
use crate::asset_resolver::BHOONIDHI_DOWNLOAD_URL;
pub use crate::asset_resolver::{
    EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID, PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
    PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID, PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
};
pub use crate::execution_policy::{
    EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID, PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID,
    PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID, PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID,
};
use crate::BHOONIDHI_CREDENTIAL_PROFILE;

pub const CDSE_SENTINEL2_L2A_POLICY_ID: &str = CDSE_SENTINEL2_L2A_RESOLVER_POLICY_ID;
pub const CDSE_SENTINEL2_L1C_POLICY_ID: &str = CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID;
pub const CDSE_SENTINEL3_STANDARD_POLICY_ID: &str = CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID;
pub const USGS_LANDSAT_C2_L2_POLICY_ID_V1: &str = USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V1;
pub const USGS_LANDSAT_C2_L2_POLICY_ID_V2: &str = USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2;
pub const USGS_LANDSAT_C2_L2_POLICY_ID: &str = USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogPolicyContext {
    OfflineFixture,
    Production,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogExecutionPolicyDefinition {
    pub policy_id: &'static str,
    pub collection_profile_id: &'static str,
    pub normalized_role: AssetRole,
    pub resolver_id: &'static str,
    pub access_scheme: AssetAccessScheme,
    pub credential_profile: &'static str,
    pub offline_only: bool,
}

static CDSE_SENTINEL2_L2A_POLICY: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: CDSE_SENTINEL2_L2A_POLICY_ID,
        collection_profile_id: SENTINEL2_L2A_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "cdse.archive.resolver.v1",
        access_scheme: AssetAccessScheme::OauthBearerHttps,
        credential_profile: "copernicus-dataspace-default",
        offline_only: false,
    };

static CDSE_SENTINEL2_L1C_POLICY: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: CDSE_SENTINEL2_L1C_POLICY_ID,
        collection_profile_id: SENTINEL2_L1C_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "cdse.archive.resolver.v1",
        access_scheme: AssetAccessScheme::OauthBearerHttps,
        credential_profile: "copernicus-dataspace-default",
        offline_only: false,
    };

static CDSE_SENTINEL3_STANDARD_POLICY: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: CDSE_SENTINEL3_STANDARD_POLICY_ID,
        collection_profile_id: SENTINEL3_STANDARD_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "cdse.archive.resolver.v1",
        access_scheme: AssetAccessScheme::OauthBearerHttps,
        credential_profile: "copernicus-dataspace-default",
        offline_only: false,
    };

static USGS_LANDSAT_C2_L2_POLICY_V1: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V1,
        collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V1,
        access_scheme: AssetAccessScheme::UsgsM2m,
        credential_profile: USGS_M2M_SESSION_PROFILE_ID,
        offline_only: false,
    };

static USGS_LANDSAT_C2_L2_POLICY_V2: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V2,
        collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V2,
        access_scheme: AssetAccessScheme::UsgsM2m,
        credential_profile: USGS_M2M_SESSION_PROFILE_ID,
        offline_only: false,
    };

static ASF_ALOS2_COMPLETE_ZIP_POLICY: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: ALOS2_RESOLVER_POLICY_ID,
        collection_profile_id: ALOS2_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "earthdata_https",
        access_scheme: AssetAccessScheme::EarthdataHttps,
        credential_profile: "earthdata-default",
        offline_only: false,
    };

static ASF_NISAR_COMPLETE_HDF5_POLICY_V1: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: NISAR_RESOLVER_POLICY_ID_V1,
        collection_profile_id: NISAR_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "earthdata_https",
        access_scheme: AssetAccessScheme::EarthdataHttps,
        credential_profile: "earthdata-default",
        offline_only: true,
    };

static ASF_NISAR_COMPLETE_HDF5_POLICY_V2: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: NISAR_RESOLVER_POLICY_ID_V2,
        collection_profile_id: NISAR_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: "earthdata_https",
        access_scheme: AssetAccessScheme::EarthdataHttps,
        credential_profile: "earthdata-default",
        offline_only: false,
    };

static BHOONIDHI_SSAR_RSLC_POLICY: CatalogExecutionPolicyDefinition =
    CatalogExecutionPolicyDefinition {
        policy_id: BHOONIDHI_RESOLVER_POLICY_ID,
        collection_profile_id: BHOONIDHI_PROFILE_ID,
        normalized_role: AssetRole::Primary,
        resolver_id: BHOONIDHI_RESOLVER_POLICY_ID,
        access_scheme: AssetAccessScheme::OauthBearerHttps,
        credential_profile: BHOONIDHI_CREDENTIAL_PROFILE,
        offline_only: false,
    };

#[derive(Clone, Debug)]
pub struct CatalogExecutionPolicyInput<'a> {
    pub policy_id: &'a str,
    pub collection_profile_id: &'a str,
    pub normalized_role: AssetRole,
    pub descriptor: &'a AssetAccessDescriptor,
}

/// Header-derived archive evidence supplied by an offline caller or fixture.
/// It contains no locator, credentials, or runtime authorization material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdseArchiveEvidence {
    pub filename_evidence: Option<ArchiveFilenameEvidence>,
    pub expected_size_bytes: Option<u64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogExecutionPolicyError {
    UnknownPolicy(String),
    OfflineOnly,
    ProfileMismatch,
    ProviderMismatch,
    DatasetMismatch,
    NativeCollectionMismatch,
    IdentityRuleMismatch,
    ProductTypeMismatch,
    DeliveryModelMismatch,
    NormalizedRoleMismatch,
    ResolverMismatch,
    SchemeMismatch,
    CredentialProfileMismatch,
    MissingStableHref,
    InvalidDescriptor(String),
    SnapshotInvalid,
    AssetOffline,
    ArchiveIdentityMismatch,
    MissingFileName,
    UnsafeFileName,
    MissingExpectedSize,
    MissingChecksum,
    FrozenEvidenceMismatch,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CatalogExecutionPolicyRegistry;

impl CatalogExecutionPolicyRegistry {
    pub fn get(&self, policy_id: &str) -> Option<&'static CatalogExecutionPolicyDefinition> {
        match policy_id {
            CDSE_SENTINEL2_L2A_POLICY_ID => Some(&CDSE_SENTINEL2_L2A_POLICY),
            CDSE_SENTINEL2_L1C_POLICY_ID => Some(&CDSE_SENTINEL2_L1C_POLICY),
            CDSE_SENTINEL3_STANDARD_POLICY_ID => Some(&CDSE_SENTINEL3_STANDARD_POLICY),
            USGS_LANDSAT_C2_L2_POLICY_ID_V1 => Some(&USGS_LANDSAT_C2_L2_POLICY_V1),
            USGS_LANDSAT_C2_L2_POLICY_ID_V2 => Some(&USGS_LANDSAT_C2_L2_POLICY_V2),
            ALOS2_RESOLVER_POLICY_ID => Some(&ASF_ALOS2_COMPLETE_ZIP_POLICY),
            NISAR_RESOLVER_POLICY_ID_V1 => Some(&ASF_NISAR_COMPLETE_HDF5_POLICY_V1),
            NISAR_RESOLVER_POLICY_ID_V2 => Some(&ASF_NISAR_COMPLETE_HDF5_POLICY_V2),
            BHOONIDHI_RESOLVER_POLICY_ID => Some(&BHOONIDHI_SSAR_RSLC_POLICY),
            _ => None,
        }
    }

    pub fn validate(
        &self,
        input: &CatalogExecutionPolicyInput<'_>,
        context: CatalogPolicyContext,
    ) -> Result<&'static CatalogExecutionPolicyDefinition, CatalogExecutionPolicyError> {
        let definition = self.get(input.policy_id).ok_or_else(|| {
            CatalogExecutionPolicyError::UnknownPolicy(input.policy_id.to_string())
        })?;
        if matches!(context, CatalogPolicyContext::Production) && definition.offline_only {
            return Err(CatalogExecutionPolicyError::OfflineOnly);
        }
        if input.collection_profile_id != definition.collection_profile_id {
            return Err(CatalogExecutionPolicyError::ProfileMismatch);
        }
        let profile = CollectionProfileRegistry
            .get(definition.collection_profile_id)
            .ok_or(CatalogExecutionPolicyError::ProfileMismatch)?;
        if profile.resolver_policy_id != definition.policy_id
            || (matches!(context, CatalogPolicyContext::Production) && !profile.production_enabled)
        {
            return Err(CatalogExecutionPolicyError::ProfileMismatch);
        }
        if input.normalized_role != definition.normalized_role {
            return Err(CatalogExecutionPolicyError::NormalizedRoleMismatch);
        }
        if input.descriptor.stable_href.is_none() {
            return Err(CatalogExecutionPolicyError::MissingStableHref);
        }
        input
            .descriptor
            .validate()
            .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;
        if input.descriptor.resolver_id != definition.resolver_id {
            return Err(CatalogExecutionPolicyError::ResolverMismatch);
        }
        if input.descriptor.scheme != definition.access_scheme {
            return Err(CatalogExecutionPolicyError::SchemeMismatch);
        }
        if input.descriptor.credential_profile.as_deref() != Some(definition.credential_profile) {
            return Err(CatalogExecutionPolicyError::CredentialProfileMismatch);
        }
        let stable_href = input
            .descriptor
            .stable_href
            .as_deref()
            .expect("stable href presence was checked above");
        match definition.policy_id {
            CDSE_SENTINEL2_L2A_POLICY_ID
            | CDSE_SENTINEL2_L1C_POLICY_ID
            | CDSE_SENTINEL3_STANDARD_POLICY_ID
                if !exact_cdse_archive_href(stable_href) =>
            {
                return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                    "CDSE archive stable_href must be the exact canonical $value locator"
                        .to_owned(),
                ));
            }
            USGS_LANDSAT_C2_L2_POLICY_ID_V1 if !exact_landsat_bundle_locator_v1(stable_href) => {
                return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                    "USGS Landsat v1 stable_href must be the exact historical full-bundle URN"
                        .to_owned(),
                ));
            }
            USGS_LANDSAT_C2_L2_POLICY_ID_V2 if !exact_landsat_bundle_locator_v2(stable_href) => {
                return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                    "USGS Landsat v2 stable_href must freeze reviewed bundle semantics".to_owned(),
                ));
            }
            BHOONIDHI_RESOLVER_POLICY_ID if stable_href != BHOONIDHI_DOWNLOAD_URL => {
                return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                    "Bhoonidhi stable_href must be the exact queryless /download origin".to_owned(),
                ));
            }
            _ => {}
        }
        if (definition.policy_id == NISAR_RESOLVER_POLICY_ID_V1
            || definition.policy_id == NISAR_RESOLVER_POLICY_ID_V2)
            && !input
                .descriptor
                .stable_href
                .as_deref()
                .is_some_and(|href| exact_nisar_primary_href(href, None))
        {
            return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                "NISAR primary stable_href must be the exact Provisional RSLC product route"
                    .to_owned(),
            ));
        }
        if matches!(
            definition.policy_id,
            CDSE_SENTINEL2_L2A_POLICY_ID
                | CDSE_SENTINEL2_L1C_POLICY_ID
                | CDSE_SENTINEL3_STANDARD_POLICY_ID
        ) && !input
            .descriptor
            .stable_href
            .as_deref()
            .is_some_and(exact_cdse_archive_href)
        {
            return Err(CatalogExecutionPolicyError::InvalidDescriptor(
                "CDSE archive stable_href must be the exact canonical $value locator".to_owned(),
            ));
        }
        Ok(definition)
    }
}

pub fn build_landsat_c2_l2_bundle_access(
    item: &SnapshotItem,
    option: &UsgsM2mFullBundleOption,
    context: CatalogPolicyContext,
) -> Result<FrozenAssetAccess, CatalogExecutionPolicyError> {
    item.validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    if !item.payload.is_empty() || item.asset_access.is_some() || item.execution_policy.is_some() {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    let profile = CollectionProfileRegistry
        .get(LANDSAT_C2_L2_SCENE_PROFILE_ID)
        .expect("the Landsat C2 L2 profile is statically registered");
    require_metadata_value(
        &item.metadata,
        "logical_dataset",
        profile.logical_dataset,
        CatalogExecutionPolicyError::DatasetMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "native_collection_id",
        profile.native_collection_id,
        CatalogExecutionPolicyError::NativeCollectionMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "collection_profile_id",
        profile.profile_id,
        CatalogExecutionPolicyError::ProfileMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "identity_rule_id",
        profile.identity_rule_id,
        CatalogExecutionPolicyError::IdentityRuleMismatch,
    )?;
    let filename_rule = metadata_string(&item.metadata, "filename_rule_id")?;
    if filename_rule != profile.filename_rule_id
        && filename_rule != USGS_LANDSAT_C2_L2_LEGACY_SCENE_FILENAME_RULE_ID
    {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    require_metadata_value(
        &item.metadata,
        "delivery_model",
        "archive_first",
        CatalogExecutionPolicyError::DeliveryModelMismatch,
    )?;
    if item.provider_id != profile.provider_id
        || option.dataset_alias != profile.native_collection_id
        || option.product_name != LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME
        || !is_supported_landsat_c2_l2_bundle_option(
            &option.option_id,
            &option.product_name,
            &option.product_code,
            &option.download_system,
        )
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let provider_product_id = metadata_string(&item.metadata, "provider_product_id")?;
    let provider_product_key_value = metadata_string(&item.metadata, "provider_product_key")?;
    let properties = item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    if properties.get("entity_id").and_then(Value::as_str) != Some(provider_product_id)
        || properties.get("landsat_product_id").and_then(Value::as_str)
            != Some(item.display_name.as_str())
        || properties.get("dataset_alias").and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || option.entity_id != provider_product_id
        || option.display_id != item.display_name
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let expected_product_key = provider_product_key(
        profile.provider_id,
        profile.native_collection_id,
        provider_product_id,
    )
    .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if item.item_id != expected_product_key || provider_product_key_value != expected_product_key {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    if option.expected_file_name != format!("{}.tar", item.display_name)
        || !safe_landsat_bundle_filename(&option.expected_file_name)
    {
        return Err(CatalogExecutionPolicyError::UnsafeFileName);
    }
    if option.expected_size_bytes.is_none_or(|size| size == 0)
        && !valid_frozen_checksum(
            option.checksum_algorithm.as_deref(),
            option.checksum_value.as_deref(),
        )
    {
        return Err(CatalogExecutionPolicyError::MissingExpectedSize);
    }
    if option.checksum_algorithm.is_some()
        && !valid_frozen_checksum(
            option.checksum_algorithm.as_deref(),
            option.checksum_value.as_deref(),
        )
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let stable_asset_key_value = stable_asset_key(&expected_product_key, &option.option_id)
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    let stable_href = landsat_bundle_locator(option);
    let descriptor = AssetAccessDescriptor {
        resolver_id: USGS_LANDSAT_C2_L2_RESOLVER_ID_V2.to_owned(),
        scheme: AssetAccessScheme::UsgsM2m,
        stable_href: Some(stable_href),
        requires_credentials: true,
        credential_profile: Some(USGS_M2M_SESSION_PROFILE_ID.to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    CatalogExecutionPolicyRegistry.validate(
        &CatalogExecutionPolicyInput {
            policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V2,
            collection_profile_id: profile.profile_id,
            normalized_role: AssetRole::Primary,
            descriptor: &descriptor,
        },
        context,
    )?;
    Ok(FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: profile.provider_id.to_owned(),
            collection_id: profile.native_collection_id.to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: expected_product_key,
            canonical_product_key: None,
            asset_key: option.option_id.clone(),
            stable_asset_key: stable_asset_key_value,
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(option.expected_file_name.clone()),
        expected_size_bytes: option.expected_size_bytes,
        checksum_algorithm: option.checksum_algorithm.clone(),
        checksum_value: option.checksum_value.clone(),
    })
}

pub fn build_planetary_computer_landsat_asset_access(
    item: &SnapshotItem,
    manifest: &PlanetaryComputerLandsatManifest,
    asset: &PlanetaryComputerLandsatAsset,
) -> Result<FrozenAssetAccess, CatalogExecutionPolicyError> {
    // Preserve the legacy USGS-tagged delivery and the explicit Microsoft v2
    // binding only. A newly registered search profile is not delivery admission.
    if !matches!(
        item.provider_id.as_str(),
        insar_provider::catalog::LANDSAT_C2_L2_PROVIDER_ID
            | insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID
    ) {
        return Err(CatalogExecutionPolicyError::ProviderMismatch);
    }
    item.validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    if !item.payload.is_empty() || item.asset_access.is_some() || item.execution_policy.is_some() {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    let profile = CollectionProfileRegistry
        .for_source(
            insar_provider::catalog::LANDSAT_C2_L2_LOGICAL_DATASET,
            &item.provider_id,
            metadata_string(&item.metadata, "native_collection_id")?,
        )
        .ok_or(CatalogExecutionPolicyError::ProviderMismatch)?;
    require_metadata_value(
        &item.metadata,
        "logical_dataset",
        profile.logical_dataset,
        CatalogExecutionPolicyError::DatasetMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "native_collection_id",
        profile.native_collection_id,
        CatalogExecutionPolicyError::NativeCollectionMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "collection_profile_id",
        profile.profile_id,
        CatalogExecutionPolicyError::ProfileMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "identity_rule_id",
        profile.identity_rule_id,
        CatalogExecutionPolicyError::IdentityRuleMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "delivery_model",
        "archive_first",
        CatalogExecutionPolicyError::DeliveryModelMismatch,
    )?;
    let provider_product_id = metadata_string(&item.metadata, "provider_product_id")?;
    let provider_product_key_value = metadata_string(&item.metadata, "provider_product_key")?;
    let properties = item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    let processing_level = properties
        .get("processing_level")
        .and_then(Value::as_str)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    if item.provider_id != profile.provider_id
        || properties.get("entity_id").and_then(Value::as_str) != Some(provider_product_id)
        || properties.get("landsat_product_id").and_then(Value::as_str)
            != Some(item.display_name.as_str())
        || properties.get("dataset_alias").and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || manifest.entity_id != provider_product_id
        || manifest.display_id != item.display_name
        || manifest.processing_level != processing_level
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let expected_product_key = provider_product_key(
        profile.provider_id,
        profile.native_collection_id,
        provider_product_id,
    )
    .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if item.item_id != expected_product_key || provider_product_key_value != expected_product_key {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    if profile.provider_id == insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID {
        insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            provider_product_id,
            &item.display_name,
        )
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
        if manifest.stac_item_id != provider_product_id {
            return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
        }
    }
    validate_planetary_computer_manifest(manifest)?;
    if manifest
        .assets
        .iter()
        .filter(|candidate| candidate.asset_key == asset.asset_key)
        .count()
        != 1
        || !manifest.assets.iter().any(|candidate| candidate == asset)
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    validate_planetary_computer_landsat_asset(
        &asset.stable_href,
        &manifest.display_id,
        &asset.asset_key,
        &asset.file_name,
        &asset.role,
    )
    .map_err(|_| CatalogExecutionPolicyError::FrozenEvidenceMismatch)?;
    let stable_asset_key_value = stable_asset_key(&expected_product_key, &asset.asset_key)
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    let descriptor = AssetAccessDescriptor {
        resolver_id: PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID.to_owned(),
        scheme: AssetAccessScheme::PublicHttps,
        stable_href: Some(asset.stable_href.clone()),
        requires_credentials: false,
        credential_profile: None,
        resolver_parameters: AssetResolverParameters {},
    };
    descriptor
        .validate()
        .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;
    Ok(FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: profile.provider_id.to_owned(),
            collection_id: profile.native_collection_id.to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: expected_product_key,
            canonical_product_key: None,
            asset_key: asset.asset_key.clone(),
            stable_asset_key: stable_asset_key_value,
        },
        descriptor,
        role: asset.role.clone(),
        file_name: Some(asset.file_name.clone()),
        expected_size_bytes: Some(asset.expected_size_bytes),
        checksum_algorithm: None,
        checksum_value: None,
    })
}

pub fn build_planetary_computer_reviewed_cog_item(
    source: &SnapshotItem,
    class: PlanetaryComputerReviewedAssetClass,
    asset_key: &str,
    stable_href: &str,
    evidence: &PlanetaryComputerReviewedAssetEvidence,
) -> Result<SnapshotItem, CatalogExecutionPolicyError> {
    source
        .validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    if source.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || !source.payload.is_empty()
        || source.asset_access.is_some()
        || source.execution_policy.is_some()
        || source.metadata.contains_key("offline_fixture_only")
    {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    let (source_kind, product_kind, product_family, processing_level, resolver_id, policy_id) =
        match class {
            PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog => (
                "sentinel1",
                "sar_grd",
                "SENTINEL1_GRD_COG",
                "GRD",
                PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
                PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
            ),
            PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog => (
                "dem",
                "dem_raster",
                "COPDEM_GLO30_COG",
                "GLO-30",
                PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
                PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID,
            ),
            PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog => (
                "sentinel2",
                "optical_scene",
                "SENTINEL2_L2A_BAND_COG",
                "L2A",
                PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
                PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID,
            ),
        };
    require_metadata_value(
        &source.metadata,
        "logical_dataset",
        class.logical_dataset(),
        CatalogExecutionPolicyError::DatasetMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "native_collection_id",
        class.collection_id(),
        CatalogExecutionPolicyError::NativeCollectionMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "collection_profile_id",
        class.collection_profile_id(),
        CatalogExecutionPolicyError::ProfileMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "identity_rule_id",
        MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
        CatalogExecutionPolicyError::IdentityRuleMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "source_kind",
        source_kind,
        CatalogExecutionPolicyError::ProductTypeMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "product_kind",
        product_kind,
        CatalogExecutionPolicyError::ProductTypeMismatch,
    )?;

    let provider_product_id = metadata_string(&source.metadata, "provider_product_id")?;
    let expected_product_key = provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        provider_product_id,
    )
    .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if source.item_id != expected_product_key
        || metadata_string(&source.metadata, "provider_product_key")? != expected_product_key
        || metadata_string(&source.metadata, "stac_item_id")? != provider_product_id
        || source.display_name != provider_product_id
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let source_assets = source
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    let descriptive = source_assets
        .iter()
        .filter_map(Value::as_object)
        .filter(|asset| asset.get("asset_key").and_then(Value::as_str) == Some(asset_key))
        .collect::<Vec<_>>();
    let [descriptive] = descriptive.as_slice() else {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    };
    let expected_stable_key = stable_asset_key(&expected_product_key, asset_key)
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if descriptive.get("stable_asset_key").and_then(Value::as_str)
        != Some(expected_stable_key.as_str())
        || descriptive.get("role").and_then(Value::as_str) != Some("primary")
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let file_name = validate_planetary_computer_reviewed_asset_identity(
        class,
        provider_product_id,
        asset_key,
        stable_href,
    )
    .map_err(|_| CatalogExecutionPolicyError::FrozenEvidenceMismatch)?;
    if evidence.expected_size_bytes == 0 || evidence.file_name != file_name {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }

    let descriptor = AssetAccessDescriptor {
        resolver_id: resolver_id.to_owned(),
        scheme: AssetAccessScheme::PublicHttps,
        stable_href: Some(stable_href.to_owned()),
        requires_credentials: false,
        credential_profile: None,
        resolver_parameters: AssetResolverParameters {},
    };
    descriptor
        .validate()
        .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID.to_owned(),
            collection_id: class.collection_id().to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: expected_product_key.clone(),
            canonical_product_key: None,
            asset_key: asset_key.to_owned(),
            stable_asset_key: expected_stable_key.clone(),
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(file_name.clone()),
        expected_size_bytes: Some(evidence.expected_size_bytes),
        checksum_algorithm: None,
        checksum_value: None,
    };
    access
        .validate()
        .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;

    let mut metadata = source.metadata.clone();
    for (key, value) in [
        ("delivery_model", "reviewed_cog"),
        ("product_family", product_family),
        ("processing_level", processing_level),
        ("source_item_id", expected_product_key.as_str()),
        ("delivery_asset_key", asset_key),
        ("delivery_asset_role", "primary"),
        ("delivery_source", "planetary_computer"),
    ] {
        metadata.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    metadata.insert(
        "delivery_expected_size_bytes".to_owned(),
        Value::Number(evidence.expected_size_bytes.into()),
    );

    let item = SnapshotItem {
        item_id: expected_stable_key,
        provider_id: source.provider_id.clone(),
        display_name: file_name,
        acquisition_datetime: source.acquisition_datetime.clone(),
        footprint_geojson: source.footprint_geojson.clone(),
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: policy_id.to_owned(),
            collection_profile_id: Some(class.collection_profile_id().to_owned()),
        }),
    };
    item.validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    Ok(item)
}

pub fn build_earth_search_sentinel2_band_cog_item(
    source: &SnapshotItem,
    asset_key: &str,
    stable_href: &str,
    evidence: &PlanetaryComputerReviewedAssetEvidence,
) -> Result<SnapshotItem, CatalogExecutionPolicyError> {
    source
        .validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    if source.provider_id != EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
        || !source.payload.is_empty()
        || source.asset_access.is_some()
        || source.execution_policy.is_some()
        || source.metadata.contains_key("offline_fixture_only")
    {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    require_metadata_value(
        &source.metadata,
        "logical_dataset",
        "optical.sentinel-2.l2a",
        CatalogExecutionPolicyError::DatasetMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "native_collection_id",
        EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
        CatalogExecutionPolicyError::NativeCollectionMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "collection_profile_id",
        EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID,
        CatalogExecutionPolicyError::ProfileMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "identity_rule_id",
        EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
        CatalogExecutionPolicyError::IdentityRuleMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "source_kind",
        "sentinel2",
        CatalogExecutionPolicyError::ProductTypeMismatch,
    )?;
    require_metadata_value(
        &source.metadata,
        "product_kind",
        "optical_scene",
        CatalogExecutionPolicyError::ProductTypeMismatch,
    )?;

    let provider_product_id = metadata_string(&source.metadata, "provider_product_id")?;
    let expected_product_key = provider_product_key(
        EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
        EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
        provider_product_id,
    )
    .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if source.item_id != expected_product_key
        || metadata_string(&source.metadata, "provider_product_key")? != expected_product_key
        || metadata_string(&source.metadata, "stac_item_id")? != provider_product_id
        || source.display_name != provider_product_id
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let source_assets = source
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    let descriptive = source_assets
        .iter()
        .filter_map(Value::as_object)
        .filter(|asset| asset.get("asset_key").and_then(Value::as_str) == Some(asset_key))
        .collect::<Vec<_>>();
    let [descriptive] = descriptive.as_slice() else {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    };
    let expected_stable_key = stable_asset_key(&expected_product_key, asset_key)
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if descriptive.get("stable_asset_key").and_then(Value::as_str)
        != Some(expected_stable_key.as_str())
        || descriptive.get("role").and_then(Value::as_str) != Some("primary")
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let file_name = validate_earth_search_sentinel2_stable_band_url(
        provider_product_id,
        asset_key,
        stable_href,
    )
    .map_err(|_| CatalogExecutionPolicyError::FrozenEvidenceMismatch)?;
    if evidence.expected_size_bytes == 0 || evidence.file_name != file_name {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }

    let descriptor = AssetAccessDescriptor {
        resolver_id: EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID.to_owned(),
        scheme: AssetAccessScheme::PublicHttps,
        stable_href: Some(stable_href.to_owned()),
        requires_credentials: false,
        credential_profile: None,
        resolver_parameters: AssetResolverParameters {},
    };
    descriptor
        .validate()
        .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID.to_owned(),
            collection_id: EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID.to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: expected_product_key.clone(),
            canonical_product_key: None,
            asset_key: asset_key.to_owned(),
            stable_asset_key: expected_stable_key.clone(),
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(file_name.clone()),
        expected_size_bytes: Some(evidence.expected_size_bytes),
        checksum_algorithm: None,
        checksum_value: None,
    };
    access
        .validate()
        .map_err(CatalogExecutionPolicyError::InvalidDescriptor)?;

    let mut metadata = source.metadata.clone();
    for (key, value) in [
        ("delivery_model", "reviewed_cog"),
        ("product_family", "SENTINEL2_L2A_BAND_COG"),
        ("processing_level", "L2A"),
        ("source_item_id", expected_product_key.as_str()),
        ("delivery_asset_key", asset_key),
        ("delivery_asset_role", "primary"),
        ("delivery_source", "earth_search"),
    ] {
        metadata.insert(key.to_owned(), Value::String(value.to_owned()));
    }
    metadata.insert(
        "delivery_expected_size_bytes".to_owned(),
        Value::Number(evidence.expected_size_bytes.into()),
    );

    let item = SnapshotItem {
        item_id: expected_stable_key,
        provider_id: source.provider_id.clone(),
        display_name: file_name,
        acquisition_datetime: source.acquisition_datetime.clone(),
        footprint_geojson: source.footprint_geojson.clone(),
        metadata,
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID.to_owned(),
            collection_profile_id: Some(EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID.to_owned()),
        }),
    };
    item.validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    Ok(item)
}

fn validate_planetary_computer_manifest(
    manifest: &PlanetaryComputerLandsatManifest,
) -> Result<(), CatalogExecutionPolicyError> {
    if !matches!(manifest.processing_level.as_str(), "l2sp" | "l2sr") {
        return Err(CatalogExecutionPolicyError::ProductTypeMismatch);
    }
    if manifest.assets.is_empty()
        || manifest.total_size_bytes == 0
        || manifest.stac_item_id.is_empty()
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let mut keys = HashSet::with_capacity(manifest.assets.len());
    let mut file_names = HashSet::with_capacity(manifest.assets.len());
    let mut total = 0_u64;
    for asset in &manifest.assets {
        if asset.expected_size_bytes == 0
            || !keys.insert(asset.asset_key.as_str())
            || !file_names.insert(asset.file_name.as_str())
            || validate_planetary_computer_landsat_asset(
                &asset.stable_href,
                &manifest.display_id,
                &asset.asset_key,
                &asset.file_name,
                &asset.role,
            )
            .is_err()
        {
            return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
        }
        total = total
            .checked_add(asset.expected_size_bytes)
            .ok_or(CatalogExecutionPolicyError::FrozenEvidenceMismatch)?;
    }
    validate_planetary_computer_landsat_manifest_asset_keys(
        &manifest.display_id,
        &manifest.processing_level,
        &keys,
    )
    .map_err(|_| CatalogExecutionPolicyError::FrozenEvidenceMismatch)?;
    if total != manifest.total_size_bytes {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    Ok(())
}

pub(crate) fn exact_nisar_primary_href(locator: &str, expected_file_name: Option<&str>) -> bool {
    let Ok(parsed) = Url::parse(locator) else {
        return false;
    };
    let Some(segments) = parsed
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
    else {
        return false;
    };
    let [root, collection, product, file_name] = segments.as_slice() else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.host_str() == Some("nisar.asf.earthdatacloud.nasa.gov")
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && *root == "NISAR"
        && *collection == NISAR_COLLECTION_SHORT_NAME
        && !product.is_empty()
        && *file_name == format!("{product}.h5")
        && expected_file_name.is_none_or(|expected| *file_name == expected)
}

/// Construct one frozen CDSE archive access object from the immutable catalogue
/// snapshot plus already-reviewed filename, size, and checksum evidence.
///
/// This remains an offline policy helper: it does not create credentials,
/// resolve runtime access, mutate a Store, or make a request.
pub fn build_cdse_archive_access(
    item: &SnapshotItem,
    evidence: CdseArchiveEvidence,
    context: CatalogPolicyContext,
) -> Result<FrozenAssetAccess, CatalogExecutionPolicyError> {
    item.validate()
        .map_err(|_| CatalogExecutionPolicyError::SnapshotInvalid)?;
    if !item.payload.is_empty() || item.asset_access.is_some() || item.execution_policy.is_some() {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    }
    let target_profile_id = match (
        item.metadata
            .get("collection_profile_id")
            .and_then(Value::as_str),
        item.metadata.get("logical_dataset").and_then(Value::as_str),
    ) {
        (Some(SENTINEL2_L1C_PROFILE_ID), _) | (_, Some("optical.sentinel-2.l1c")) => {
            SENTINEL2_L1C_PROFILE_ID
        }
        (Some(SENTINEL3_STANDARD_PROFILE_ID), _) | (_, Some("optical.sentinel-3.standard")) => {
            SENTINEL3_STANDARD_PROFILE_ID
        }
        _ => SENTINEL2_L2A_PROFILE_ID,
    };
    let profile = CollectionProfileRegistry
        .get(target_profile_id)
        .expect("the reviewed CDSE Sentinel profile is statically registered");
    if item.provider_id != CDSE_PROVIDER_ID || item.provider_id != profile.provider_id {
        return Err(CatalogExecutionPolicyError::ProviderMismatch);
    }
    require_metadata_value(
        &item.metadata,
        "logical_dataset",
        profile.logical_dataset,
        CatalogExecutionPolicyError::DatasetMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "native_collection_id",
        profile.native_collection_id,
        CatalogExecutionPolicyError::NativeCollectionMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "collection_profile_id",
        profile.profile_id,
        CatalogExecutionPolicyError::ProfileMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "identity_rule_id",
        profile.identity_rule_id,
        CatalogExecutionPolicyError::IdentityRuleMismatch,
    )?;
    require_metadata_value(
        &item.metadata,
        "filename_rule_id",
        profile.filename_rule_id,
        CatalogExecutionPolicyError::SnapshotInvalid,
    )?;
    require_metadata_value(
        &item.metadata,
        "delivery_model",
        "archive_first",
        CatalogExecutionPolicyError::DeliveryModelMismatch,
    )?;

    let provider_product_id = metadata_string(&item.metadata, "provider_product_id")?;
    if !canonical_cdse_odata_uuid(provider_product_id) {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let expected_product_key = provider_product_key(
        profile.provider_id,
        profile.native_collection_id,
        provider_product_id,
    )
    .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    if item.item_id != expected_product_key
        || metadata_string(&item.metadata, "provider_product_key")? != expected_product_key
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }
    let expected_asset_key = stable_asset_key(&expected_product_key, CDSE_ARCHIVE_ASSET_KEY)
        .map_err(|_| CatalogExecutionPolicyError::ArchiveIdentityMismatch)?;
    let properties = item
        .metadata
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?;
    let product_type_ok = match (
        profile.profile_id,
        properties.get("product_type").and_then(Value::as_str),
    ) {
        (SENTINEL3_STANDARD_PROFILE_ID, Some(product_type)) => matches!(
            product_type,
            "OL_1_EFR___"
                | "OL_1_ERR___"
                | "OL_2_LFR___"
                | "OL_2_LRR___"
                | "SL_1_RBT___"
                | "SL_2_LST___"
                | "SR_1_SRA___"
                | "SR_2_LAN___"
                | "OL_1_EFR"
        ),
        (_, actual) => actual == Some(profile.required_product_type),
    };
    if !product_type_ok {
        return Err(CatalogExecutionPolicyError::ProductTypeMismatch);
    }
    if properties.get("online").and_then(Value::as_bool) != Some(true) {
        return Err(CatalogExecutionPolicyError::AssetOffline);
    }
    let archive_assets = item
        .metadata
        .get("assets")
        .and_then(Value::as_array)
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)?
        .iter()
        .filter_map(Value::as_object)
        .filter(|asset| {
            asset.get("asset_key").and_then(Value::as_str) == Some(CDSE_ARCHIVE_ASSET_KEY)
        })
        .collect::<Vec<_>>();
    let [asset] = archive_assets.as_slice() else {
        return Err(CatalogExecutionPolicyError::SnapshotInvalid);
    };
    if asset.get("asset_key").and_then(Value::as_str) != Some(CDSE_ARCHIVE_ASSET_KEY)
        || asset.get("stable_asset_key").and_then(Value::as_str)
            != Some(expected_asset_key.as_str())
        || asset.get("role").and_then(Value::as_str) != Some("primary")
        || asset
            .get("native_roles")
            .and_then(Value::as_array)
            .is_none_or(|roles| {
                roles.len() != 1 || roles.first().and_then(Value::as_str) != Some("archive")
            })
    {
        return Err(CatalogExecutionPolicyError::ArchiveIdentityMismatch);
    }

    let filename_evidence = evidence
        .filename_evidence
        .ok_or(CatalogExecutionPolicyError::MissingFileName)?;
    if filename_evidence.rule_id != profile.filename_rule_id
        || filename_evidence.odata_name != item.display_name
        || !safe_cdse_odata_name(&filename_evidence.odata_name)
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let file_name = filename_evidence.basename;
    if !safe_cdse_archive_basename(&file_name) {
        return Err(CatalogExecutionPolicyError::UnsafeFileName);
    }
    if file_name != format!("{}.zip", item.display_name) {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let expected_size_bytes = evidence
        .expected_size_bytes
        .filter(|size| *size > 0)
        .ok_or(CatalogExecutionPolicyError::MissingExpectedSize)?;
    if asset
        .get("size_bytes")
        .is_some_and(|value| value.as_u64() != Some(expected_size_bytes))
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }
    let checksum_algorithm = evidence
        .checksum_algorithm
        .ok_or(CatalogExecutionPolicyError::MissingChecksum)?;
    let checksum_value = evidence
        .checksum_value
        .ok_or(CatalogExecutionPolicyError::MissingChecksum)?;
    if !valid_cdse_checksum(Some(&checksum_algorithm), Some(&checksum_value))
        || !metadata_has_checksum(properties, &checksum_algorithm, &checksum_value)
    {
        return Err(CatalogExecutionPolicyError::FrozenEvidenceMismatch);
    }

    let reference = StableAssetReference {
        provider_id: CDSE_PROVIDER_ID.to_owned(),
        collection_id: profile.native_collection_id.to_owned(),
        provider_product_id: provider_product_id.to_owned(),
        provider_product_key: expected_product_key,
        canonical_product_key: None,
        asset_key: CDSE_ARCHIVE_ASSET_KEY.to_owned(),
        stable_asset_key: expected_asset_key,
    };
    let descriptor = AssetAccessDescriptor {
        resolver_id: CDSE_ARCHIVE_RESOLVER_ID.to_owned(),
        scheme: AssetAccessScheme::OauthBearerHttps,
        stable_href: stable_cdse_archive_href(provider_product_id),
        requires_credentials: true,
        credential_profile: Some(CDSE_CREDENTIAL_PROFILE.to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    CatalogExecutionPolicyRegistry.validate(
        &CatalogExecutionPolicyInput {
            policy_id: profile.resolver_policy_id,
            collection_profile_id: profile.profile_id,
            normalized_role: AssetRole::Primary,
            descriptor: &descriptor,
        },
        context,
    )?;
    Ok(FrozenAssetAccess {
        reference,
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(file_name),
        expected_size_bytes: Some(expected_size_bytes),
        checksum_algorithm: Some(checksum_algorithm),
        checksum_value: Some(checksum_value),
    })
}

/// Compatibility helper retained for P2 fixtures and the ignored live harness.
/// New production freeze paths must call `build_cdse_archive_access` with an
/// explicit `CatalogPolicyContext::Production`.
pub fn build_offline_cdse_archive_access(
    item: &SnapshotItem,
    evidence: CdseArchiveEvidence,
) -> Result<FrozenAssetAccess, CatalogExecutionPolicyError> {
    build_cdse_archive_access(item, evidence, CatalogPolicyContext::OfflineFixture)
}

fn require_metadata_value(
    metadata: &Map<String, Value>,
    key: &str,
    expected: &str,
    error: CatalogExecutionPolicyError,
) -> Result<(), CatalogExecutionPolicyError> {
    (metadata.get(key).and_then(Value::as_str) == Some(expected))
        .then_some(())
        .ok_or(error)
}

fn metadata_string<'a>(
    metadata: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, CatalogExecutionPolicyError> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && *value == value.trim())
        .ok_or(CatalogExecutionPolicyError::SnapshotInvalid)
}

fn metadata_has_checksum(properties: &Map<String, Value>, algorithm: &str, value: &str) -> bool {
    properties
        .get("checksums")
        .and_then(Value::as_array)
        .is_some_and(|checksums| {
            checksums.iter().any(|checksum| {
                checksum.get("algorithm").and_then(Value::as_str) == Some(algorithm)
                    && checksum.get("value").and_then(Value::as_str) == Some(value)
            })
        })
}

fn landsat_bundle_locator(option: &UsgsM2mFullBundleOption) -> String {
    format!(
        "urn:usgs:m2m:landsat:{}:{}:full_bundle:{}:{}:{}:{}:{}",
        option.dataset_alias,
        option.entity_id,
        option.option_id,
        option.availability.as_str(),
        option.display_id,
        option.product_code,
        option.download_system,
    )
}

pub(crate) fn exact_landsat_bundle_locator_v1(value: &str) -> bool {
    parse_frozen_landsat_bundle_locator_v1(value).is_some()
}

pub(crate) fn exact_landsat_bundle_locator_v2(value: &str) -> bool {
    parse_frozen_landsat_bundle_locator_v2(value).is_some()
}

fn safe_landsat_bundle_filename(value: &str) -> bool {
    value.len() <= 128
        && value.ends_with(".tar")
        && value != ".tar"
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn valid_frozen_checksum(algorithm: Option<&str>, value: Option<&str>) -> bool {
    let (Some(algorithm), Some(value)) = (algorithm, value) else {
        return false;
    };
    let expected = match algorithm.to_ascii_lowercase().as_str() {
        "md5" => 32,
        "sha1" => 40,
        "sha256" => 64,
        "sha512" => 128,
        _ => return false,
    };
    value.len() == expected && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
