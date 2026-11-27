mod alos2;
mod bhoonidhi;
use bhoonidhi::{evaluate_bhoonidhi_resolver_cutover_eligibility, BHOONIDHI_SSAR_COMPLETE_POLICY};
mod burst;
mod nisar;

use alos2::{evaluate_alos2_resolver_cutover_eligibility, ASF_ALOS2_COMPLETE_ZIP_POLICY};
use burst::evaluate_burst_resolver_cutover_eligibility;
pub use burst::{
    build_burst_frozen_asset_access, ASF_BURST_ASSET_EXECUTION_POLICY_ID,
    ASF_BURST_COLLECTION_PROFILE_ID, EARTHDATA_BURST_CREDENTIAL_PROFILE,
};
use insar_core::{
    stable_asset_key, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy, AssetRole,
    FrozenAssetAccess, SnapshotItem,
};
use insar_provider::asf::collection_profile::{
    profile_for_id, AsfCollectionProfile, AsfDeliveryModel,
};
use insar_provider::catalog::{
    valid_mpc_hls_scene_id, validate_earth_search_sentinel2_stable_band_url,
    CollectionProfileRegistry, MpcHlsProduct, ALOS2_RESOLVER_POLICY_ID,
    CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID,
    CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID as PROVIDER_CDSE_SENTINEL2_L1C_POLICY_ID,
    CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID,
    CDSE_SENTINEL2_L2A_RESOLVER_POLICY_ID as PROVIDER_CDSE_SENTINEL2_L2A_POLICY_ID,
    CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID,
    CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID as PROVIDER_CDSE_SENTINEL3_STANDARD_POLICY_ID,
    EARTH_SEARCH_SENTINEL2_L2A_BAND_ASSET_POLICY_ID, EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID, EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID, LANDSAT_C2_L2_LOGICAL_DATASET,
    LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_SCENE_PROFILE_ID,
    MPC_HLS_IDENTITY_RULE_ID, MPC_HLS_L30_ASSET_POLICY_ID, MPC_HLS_L30_PROFILE_ID,
    MPC_HLS_PROVIDER_ID, MPC_HLS_S30_ASSET_POLICY_ID, MPC_HLS_S30_PROFILE_ID,
    NISAR_RESOLVER_POLICY_ID_V1, NISAR_RESOLVER_POLICY_ID_V2,
    SENTINEL2_L1C_PROFILE_ID as PROVIDER_CDSE_SENTINEL2_L1C_PROFILE_ID,
    SENTINEL2_L2A_PROFILE_ID as PROVIDER_CDSE_SENTINEL2_L2A_PROFILE_ID,
    SENTINEL3_STANDARD_PROFILE_ID as PROVIDER_CDSE_SENTINEL3_STANDARD_PROFILE_ID,
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID,
};
use insar_provider::planetary_computer::validate_planetary_computer_landsat_asset;
use insar_provider::planetary_computer_asset::{
    validate_planetary_computer_reviewed_asset_identity, PlanetaryComputerReviewedAssetClass,
    MPC_COPDEM_GLO30_ASSET_POLICY_ID, MPC_COPDEM_GLO30_PROFILE_ID,
    MPC_REVIEWED_STAC_IDENTITY_RULE_ID, MPC_SENTINEL1_GRD_ASSET_POLICY_ID,
    MPC_SENTINEL1_GRD_PROFILE_ID, MPC_SENTINEL2_L2A_BAND_ASSET_POLICY_ID,
    MPC_SENTINEL2_L2A_BAND_PROFILE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use insar_provider::usgs_m2m::USGS_M2M_SESSION_PROFILE_ID;
use nisar::{
    evaluate_nisar_resolver_cutover_eligibility, ASF_NISAR_COMPLETE_HDF5_POLICY_V1,
    ASF_NISAR_COMPLETE_HDF5_POLICY_V2,
};
use serde_json::Value;

use crate::asset_resolver::cdse::{
    canonical_cdse_odata_uuid, safe_cdse_archive_basename, stable_cdse_archive_href,
    valid_cdse_checksum, CDSE_ARCHIVE_ASSET_KEY, CDSE_ARCHIVE_RESOLVER_ID, CDSE_CREDENTIAL_PROFILE,
    CDSE_PROVIDER_ID,
};
use crate::asset_resolver::usgs_m2m::{
    parse_frozen_landsat_bundle_locator_for_resolver, USGS_LANDSAT_C2_L2_RESOLVER_ID,
    USGS_LANDSAT_C2_L2_RESOLVER_ID_V1, USGS_LANDSAT_C2_L2_RESOLVER_ID_V2,
};
use crate::asset_resolver::{
    is_valid_planetary_computer_hls_href, EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID,
    PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID, PLANETARY_COMPUTER_HLS_RESOLVER_ID,
    PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID, PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
    PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
};
use crate::execution_policy_registry::{
    USGS_LANDSAT_C2_L2_POLICY_ID_V1, USGS_LANDSAT_C2_L2_POLICY_ID_V2,
};
use crate::{ArtifactValidationPlan, ArtifactValidatorBinding, ArtifactValidatorRegistry};

pub const ASF_SENTINEL1_STANDARD_POLICY_ID: &str = "asf.sentinel-1.standard.resolver-cutover.v1";
pub const ASF_SENTINEL1_STANDARD_PROFILE_ID: &str = "asf.sentinel-1.standard.v1";
pub const APPROVED_PRODUCT_FAMILIES: &[&str] = &["SLC", "GRD"];
pub const APPROVED_PROCESSING_LEVELS: &[&str] =
    &["SLC", "GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS", "GRD_FD"];
pub const APPROVED_RESOLVER_IDS: &[&str] = &["earthdata_https"];
pub const APPROVED_CREDENTIAL_PROFILES: &[&str] = &["earthdata-default"];
pub const CDSE_SENTINEL2_L2A_POLICY_ID: &str = PROVIDER_CDSE_SENTINEL2_L2A_POLICY_ID;
pub const CDSE_SENTINEL2_L2A_PROFILE_ID: &str = PROVIDER_CDSE_SENTINEL2_L2A_PROFILE_ID;
pub const CDSE_SENTINEL2_L1C_POLICY_ID: &str = PROVIDER_CDSE_SENTINEL2_L1C_POLICY_ID;
pub const CDSE_SENTINEL2_L1C_PROFILE_ID: &str = PROVIDER_CDSE_SENTINEL2_L1C_PROFILE_ID;
pub const CDSE_SENTINEL3_STANDARD_POLICY_ID: &str = PROVIDER_CDSE_SENTINEL3_STANDARD_POLICY_ID;
pub const CDSE_SENTINEL3_STANDARD_PROFILE_ID: &str = PROVIDER_CDSE_SENTINEL3_STANDARD_PROFILE_ID;
pub const CDSE_APPROVED_RESOLVER_IDS: &[&str] = &[CDSE_ARCHIVE_RESOLVER_ID];
pub const CDSE_APPROVED_CREDENTIAL_PROFILES: &[&str] = &[CDSE_CREDENTIAL_PROFILE];
const LANDSAT_APPROVED_RESOLVER_IDS_V1: &[&str] = &[USGS_LANDSAT_C2_L2_RESOLVER_ID_V1];
pub const LANDSAT_APPROVED_RESOLVER_IDS: &[&str] = &[USGS_LANDSAT_C2_L2_RESOLVER_ID];
pub const LANDSAT_APPROVED_CREDENTIAL_PROFILES: &[&str] = &[USGS_M2M_SESSION_PROFILE_ID];
pub const PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID: &str =
    "planetary-computer.landsat-c2-l2.asset-set.v1";
// v1 remains the immutable policy for historical USGS-tagged snapshots.
use insar_provider::catalog::{MPC_LANDSAT_ASSET_POLICY_ID, MPC_LANDSAT_SCENE_PROFILE_ID};
pub const PLANETARY_COMPUTER_LANDSAT_APPROVED_RESOLVER_IDS: &[&str] =
    &[PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID];
pub const PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID: &str = MPC_SENTINEL1_GRD_ASSET_POLICY_ID;
pub const PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID: &str = MPC_COPDEM_GLO30_ASSET_POLICY_ID;
pub const PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID: &str =
    MPC_SENTINEL2_L2A_BAND_ASSET_POLICY_ID;
pub const EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID: &str =
    EARTH_SEARCH_SENTINEL2_L2A_BAND_ASSET_POLICY_ID;
pub const PLANETARY_COMPUTER_HLS2_L30_POLICY_ID: &str = MPC_HLS_L30_ASSET_POLICY_ID;
pub const PLANETARY_COMPUTER_HLS2_S30_POLICY_ID: &str = MPC_HLS_S30_ASSET_POLICY_ID;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionDeliveryModel {
    Asf(AsfDeliveryModel),
    CdseArchiveFirst,
    UsgsM2mLandsatBundle,
    PlanetaryComputerLandsatAssets,
    PlanetaryComputerHlsAssets,
    PlanetaryComputerReviewedCog,
    EarthSearchSentinel2BandCog,
    Alos2ArchiveFirst,
    NisarArchiveFirst,
    NisarSsarArchiveFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthRefreshPolicyId {
    EarthdataStandardV1,
    EarthdataBurstSessionV1,
    /// CDSE archive Bearer semantics. This is an internal transport
    /// classification only; it does not register a resolver policy.
    CdseOAuthBearerV1,
    /// Bhoonidhi accepts one generation-bound access-token refresh after HTTP 401.
    BhoonidhiOAuthBearerV1,
    /// A USGS M2M download URL is runtime-only and may be re-resolved once
    /// after one approved-host authorization rejection.
    UsgsM2mSignedUrlV1,
    /// Anonymous Planetary Computer SAS material is runtime-only and may be
    /// reacquired once after one approved-host authorization rejection.
    PlanetaryComputerSasV1,
    /// Unsigned public HTTPS objects (e.g. AWS Open Data `sentinel-cogs`)
    /// require no credentials or runtime token refresh.
    PublicAnonymousHttpsV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetExecutionPolicyDefinition {
    pub policy_id: &'static str,
    pub collection_profile_id: &'static str,
    pub delivery_model: ExecutionDeliveryModel,
    pub approved_product_families: &'static [&'static str],
    pub approved_processing_levels: &'static [&'static str],
    pub approved_resolver_ids: &'static [&'static str],
    pub approved_credential_profiles: &'static [&'static str],
    pub approved_asset_roles: &'static [AssetRole],
    pub auth_refresh_policy_id: AuthRefreshPolicyId,
    artifact_validator_binding: Option<ArtifactValidatorBinding>,
}

impl AssetExecutionPolicyDefinition {
    pub const fn artifact_validator_binding(&self) -> Option<ArtifactValidatorBinding> {
        self.artifact_validator_binding
    }
}

static ASF_SENTINEL1_STANDARD_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: ASF_SENTINEL1_STANDARD_POLICY_ID,
        collection_profile_id: ASF_SENTINEL1_STANDARD_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::Asf(AsfDeliveryModel::SinglePrimaryArchive),
        approved_product_families: APPROVED_PRODUCT_FAMILIES,
        approved_processing_levels: APPROVED_PROCESSING_LEVELS,
        approved_resolver_ids: APPROVED_RESOLVER_IDS,
        approved_credential_profiles: APPROVED_CREDENTIAL_PROFILES,
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::EarthdataStandardV1,
        artifact_validator_binding: None,
    };

static ASF_SENTINEL1_BURST_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: ASF_BURST_ASSET_EXECUTION_POLICY_ID,
        collection_profile_id: ASF_BURST_COLLECTION_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::Asf(AsfDeliveryModel::MultiAssetProduct),
        approved_product_families: &["BURST"],
        approved_processing_levels: &["L1"],
        approved_resolver_ids: &["earthdata_https"],
        approved_credential_profiles: &[EARTHDATA_BURST_CREDENTIAL_PROFILE],
        approved_asset_roles: &[AssetRole::Primary, AssetRole::Metadata],
        auth_refresh_policy_id: AuthRefreshPolicyId::EarthdataBurstSessionV1,
        artifact_validator_binding: None,
    };

static CDSE_SENTINEL2_L2A_POLICY: AssetExecutionPolicyDefinition = AssetExecutionPolicyDefinition {
    policy_id: CDSE_SENTINEL2_L2A_POLICY_ID,
    collection_profile_id: CDSE_SENTINEL2_L2A_PROFILE_ID,
    delivery_model: ExecutionDeliveryModel::CdseArchiveFirst,
    approved_product_families: &["S2MSI2A"],
    approved_processing_levels: &["L2A"],
    approved_resolver_ids: CDSE_APPROVED_RESOLVER_IDS,
    approved_credential_profiles: CDSE_APPROVED_CREDENTIAL_PROFILES,
    approved_asset_roles: &[AssetRole::Primary],
    auth_refresh_policy_id: AuthRefreshPolicyId::CdseOAuthBearerV1,
    artifact_validator_binding: None,
};

static CDSE_SENTINEL2_L1C_POLICY: AssetExecutionPolicyDefinition = AssetExecutionPolicyDefinition {
    policy_id: CDSE_SENTINEL2_L1C_POLICY_ID,
    collection_profile_id: CDSE_SENTINEL2_L1C_PROFILE_ID,
    delivery_model: ExecutionDeliveryModel::CdseArchiveFirst,
    approved_product_families: &["S2MSI1C"],
    approved_processing_levels: &["L1C"],
    approved_resolver_ids: CDSE_APPROVED_RESOLVER_IDS,
    approved_credential_profiles: CDSE_APPROVED_CREDENTIAL_PROFILES,
    approved_asset_roles: &[AssetRole::Primary],
    auth_refresh_policy_id: AuthRefreshPolicyId::CdseOAuthBearerV1,
    artifact_validator_binding: None,
};

static CDSE_SENTINEL3_STANDARD_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: CDSE_SENTINEL3_STANDARD_POLICY_ID,
        collection_profile_id: CDSE_SENTINEL3_STANDARD_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::CdseArchiveFirst,
        approved_product_families: &[
            "OL_1_EFR___",
            "OL_1_ERR___",
            "OL_2_LFR___",
            "OL_2_LRR___",
            "SL_1_RBT___",
            "SL_2_LST___",
            "SR_1_SRA___",
            "SR_2_LAN___",
            "OL_1_EFR",
        ],
        approved_processing_levels: &["L1", "L2", "STANDARD"],
        approved_resolver_ids: CDSE_APPROVED_RESOLVER_IDS,
        approved_credential_profiles: CDSE_APPROVED_CREDENTIAL_PROFILES,
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::CdseOAuthBearerV1,
        artifact_validator_binding: None,
    };

static USGS_LANDSAT_C2_L2_POLICY_V1: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V1,
        collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::UsgsM2mLandsatBundle,
        approved_product_families: &["LANDSAT_C2_L2"],
        approved_processing_levels: &["l2sp", "l2sr"],
        approved_resolver_ids: LANDSAT_APPROVED_RESOLVER_IDS_V1,
        approved_credential_profiles: LANDSAT_APPROVED_CREDENTIAL_PROFILES,
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        artifact_validator_binding: None,
    };

static USGS_LANDSAT_C2_L2_POLICY_V2: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: USGS_LANDSAT_C2_L2_POLICY_ID_V2,
        collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::UsgsM2mLandsatBundle,
        approved_product_families: &["LANDSAT_C2_L2"],
        approved_processing_levels: &["l2sp", "l2sr"],
        approved_resolver_ids: LANDSAT_APPROVED_RESOLVER_IDS,
        approved_credential_profiles: LANDSAT_APPROVED_CREDENTIAL_PROFILES,
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID,
        collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerLandsatAssets,
        approved_product_families: &["LANDSAT_C2_L2"],
        approved_processing_levels: &["l2sp", "l2sr"],
        approved_resolver_ids: PLANETARY_COMPUTER_LANDSAT_APPROVED_RESOLVER_IDS,
        approved_credential_profiles: &[],
        approved_asset_roles: &[
            AssetRole::Primary,
            AssetRole::Metadata,
            AssetRole::Auxiliary,
        ],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

#[derive(Clone, Copy, Debug, Default)]
pub struct AsfCollectionProfileRegistry;

static MPC_NATIVE_LANDSAT_C2_L2_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: MPC_LANDSAT_ASSET_POLICY_ID,
        collection_profile_id: MPC_LANDSAT_SCENE_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerLandsatAssets,
        approved_product_families: &["LANDSAT_C2_L2"],
        approved_processing_levels: &["l2sp", "l2sr"],
        approved_resolver_ids: PLANETARY_COMPUTER_LANDSAT_APPROVED_RESOLVER_IDS,
        approved_credential_profiles: &[],
        approved_asset_roles: &[
            AssetRole::Primary,
            AssetRole::Metadata,
            AssetRole::Auxiliary,
        ],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID,
        collection_profile_id: MPC_SENTINEL1_GRD_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerReviewedCog,
        approved_product_families: &["SENTINEL1_GRD_COG"],
        approved_processing_levels: &["GRD"],
        approved_resolver_ids: &[PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID,
        collection_profile_id: MPC_COPDEM_GLO30_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerReviewedCog,
        approved_product_families: &["COPDEM_GLO30_COG"],
        approved_processing_levels: &["GLO-30"],
        approved_resolver_ids: &[PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID,
        collection_profile_id: MPC_SENTINEL2_L2A_BAND_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerReviewedCog,
        approved_product_families: &["SENTINEL2_L2A_BAND_COG"],
        approved_processing_levels: &["L2A"],
        approved_resolver_ids: &[PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

static EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID,
        collection_profile_id: EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::EarthSearchSentinel2BandCog,
        approved_product_families: &["SENTINEL2_L2A_BAND_COG"],
        approved_processing_levels: &["L2A"],
        approved_resolver_ids: &[EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[AssetRole::Primary],
        auth_refresh_policy_id: AuthRefreshPolicyId::PublicAnonymousHttpsV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_HLS2_L30_ASSET_SET_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_HLS2_L30_POLICY_ID,
        collection_profile_id: MPC_HLS_L30_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerHlsAssets,
        approved_product_families: &["HLSL30"],
        approved_processing_levels: &["v2.0"],
        approved_resolver_ids: &[PLANETARY_COMPUTER_HLS_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[
            AssetRole::Primary,
            AssetRole::Metadata,
            AssetRole::Auxiliary,
        ],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

static PLANETARY_COMPUTER_HLS2_S30_ASSET_SET_POLICY: AssetExecutionPolicyDefinition =
    AssetExecutionPolicyDefinition {
        policy_id: PLANETARY_COMPUTER_HLS2_S30_POLICY_ID,
        collection_profile_id: MPC_HLS_S30_PROFILE_ID,
        delivery_model: ExecutionDeliveryModel::PlanetaryComputerHlsAssets,
        approved_product_families: &["HLSS30"],
        approved_processing_levels: &["v2.0"],
        approved_resolver_ids: &[PLANETARY_COMPUTER_HLS_RESOLVER_ID],
        approved_credential_profiles: &[],
        approved_asset_roles: &[
            AssetRole::Primary,
            AssetRole::Metadata,
            AssetRole::Auxiliary,
        ],
        auth_refresh_policy_id: AuthRefreshPolicyId::PlanetaryComputerSasV1,
        artifact_validator_binding: None,
    };

impl AsfCollectionProfileRegistry {
    pub fn get(&self, profile_id: &str) -> Option<&'static AsfCollectionProfile> {
        profile_for_id(profile_id)
    }

    fn has_resolver_profile(&self, profile_id: &str) -> bool {
        self.get(profile_id).is_some()
            || profile_id == CDSE_SENTINEL2_L2A_PROFILE_ID
            || profile_id == LANDSAT_C2_L2_SCENE_PROFILE_ID
            || profile_id == MPC_SENTINEL1_GRD_PROFILE_ID
            || profile_id == MPC_COPDEM_GLO30_PROFILE_ID
            || profile_id == MPC_SENTINEL2_L2A_BAND_PROFILE_ID
            || profile_id == EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID
            || CollectionProfileRegistry.get(profile_id).is_some()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AssetExecutionPolicyRegistry;

impl AssetExecutionPolicyRegistry {
    pub fn get(&self, policy_id: &str) -> Option<&'static AssetExecutionPolicyDefinition> {
        match policy_id {
            ASF_SENTINEL1_STANDARD_POLICY_ID => Some(&ASF_SENTINEL1_STANDARD_POLICY),
            ASF_BURST_ASSET_EXECUTION_POLICY_ID => Some(&ASF_SENTINEL1_BURST_POLICY),
            CDSE_SENTINEL2_L2A_POLICY_ID => Some(&CDSE_SENTINEL2_L2A_POLICY),
            CDSE_SENTINEL2_L1C_POLICY_ID => Some(&CDSE_SENTINEL2_L1C_POLICY),
            CDSE_SENTINEL3_STANDARD_POLICY_ID => Some(&CDSE_SENTINEL3_STANDARD_POLICY),
            USGS_LANDSAT_C2_L2_POLICY_ID_V1 => Some(&USGS_LANDSAT_C2_L2_POLICY_V1),
            USGS_LANDSAT_C2_L2_POLICY_ID_V2 => Some(&USGS_LANDSAT_C2_L2_POLICY_V2),
            PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY)
            }
            MPC_LANDSAT_ASSET_POLICY_ID => Some(&MPC_NATIVE_LANDSAT_C2_L2_POLICY),
            PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY)
            }
            PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY)
            }
            PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY)
            }
            EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID => {
                Some(&EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY)
            }
            PLANETARY_COMPUTER_HLS2_L30_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_HLS2_L30_ASSET_SET_POLICY)
            }
            PLANETARY_COMPUTER_HLS2_S30_POLICY_ID => {
                Some(&PLANETARY_COMPUTER_HLS2_S30_ASSET_SET_POLICY)
            }
            ALOS2_RESOLVER_POLICY_ID => Some(&ASF_ALOS2_COMPLETE_ZIP_POLICY),
            NISAR_RESOLVER_POLICY_ID_V1 => Some(&ASF_NISAR_COMPLETE_HDF5_POLICY_V1),
            NISAR_RESOLVER_POLICY_ID_V2 => Some(&ASF_NISAR_COMPLETE_HDF5_POLICY_V2),
            insar_provider::bhoonidhi::BHOONIDHI_RESOLVER_POLICY_ID => {
                Some(&BHOONIDHI_SSAR_COMPLETE_POLICY)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionPolicyError {
    Structural(String),
    UnknownLegacyProfile(String),
    UnknownResolverPolicy(String),
    ResolverProfileMismatch,
    MissingAssetAccess,
    InvalidAssetAccess(String),
    MissingIdentity(&'static str),
    IdentityMismatch(&'static str),
    InvalidFilename,
    InvalidStableAssetKey,
    ArtifactValidationUnavailable,
}

impl ExecutionPolicyError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Structural(_) => "execution_policy_structural",
            Self::UnknownLegacyProfile(_) => "legacy_profile_unknown",
            Self::UnknownResolverPolicy(_) => "resolver_policy_unknown",
            Self::ResolverProfileMismatch => "resolver_profile_mismatch",
            Self::MissingAssetAccess => "asset_access_missing",
            Self::InvalidAssetAccess(_) => "asset_access_invalid",
            Self::MissingIdentity(_) => "asset_identity_missing",
            Self::IdentityMismatch(_) => "asset_identity_mismatch",
            Self::InvalidFilename => "asset_filename_invalid",
            Self::InvalidStableAssetKey => "stable_asset_key_invalid",
            Self::ArtifactValidationUnavailable => "NISAR_HDF5_VALIDATION_UNAVAILABLE",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ValidatedExecutionPolicy {
    MissingPolicyCompatibility,
    ExplicitLegacy(Option<&'static AsfCollectionProfile>),
    ResolverAuthoritative(&'static AssetExecutionPolicyDefinition),
}

pub fn validate_execution_policy(
    policy: Option<&AssetExecutionPolicy>,
    profiles: &AsfCollectionProfileRegistry,
    policies: &AssetExecutionPolicyRegistry,
) -> Result<ValidatedExecutionPolicy, ExecutionPolicyError> {
    let Some(policy) = policy else {
        return Ok(ValidatedExecutionPolicy::MissingPolicyCompatibility);
    };
    policy
        .validate()
        .map_err(ExecutionPolicyError::Structural)?;
    match policy.authority {
        AssetExecutionAuthority::Legacy => {
            let profile = match policy.collection_profile_id.as_deref() {
                Some(profile_id) => Some(profiles.get(profile_id).ok_or_else(|| {
                    ExecutionPolicyError::UnknownLegacyProfile(profile_id.to_string())
                })?),
                None => None,
            };
            Ok(ValidatedExecutionPolicy::ExplicitLegacy(profile))
        }
        AssetExecutionAuthority::ResolverAuthoritative => {
            let definition = policies.get(&policy.policy_id).ok_or_else(|| {
                ExecutionPolicyError::UnknownResolverPolicy(policy.policy_id.clone())
            })?;
            if policy.collection_profile_id.as_deref() != Some(definition.collection_profile_id)
                || !profiles.has_resolver_profile(definition.collection_profile_id)
            {
                return Err(ExecutionPolicyError::ResolverProfileMismatch);
            }
            Ok(ValidatedExecutionPolicy::ResolverAuthoritative(definition))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EligibilityReason {
    ExecutionAuthority,
    Provider,
    CollectionProfile,
    IdentityRule,
    Dataset,
    NativeCollection,
    DeliveryModel,
    ProcessingLevel,
    ProductFamily,
    ProductType,
    Online,
    AssetRole,
    AccessScheme,
    ResolverId,
    CredentialProfile,
}

impl EligibilityReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::ExecutionAuthority => "execution_authority",
            Self::Provider => "provider",
            Self::CollectionProfile => "collection_profile",
            Self::IdentityRule => "identity_rule",
            Self::Dataset => "dataset",
            Self::NativeCollection => "native_collection",
            Self::DeliveryModel => "delivery_model",
            Self::ProcessingLevel => "processing_level",
            Self::ProductFamily => "product_family",
            Self::ProductType => "product_type",
            Self::Online => "online",
            Self::AssetRole => "asset_role",
            Self::AccessScheme => "access_scheme",
            Self::ResolverId => "resolver_id",
            Self::CredentialProfile => "credential_profile",
        }
    }
}

#[derive(Clone, Debug)]
pub struct EligibleResolverAsset {
    pub frozen_access: FrozenAssetAccess,
    pub policy: &'static AssetExecutionPolicyDefinition,
    artifact_validation_plan: Option<ArtifactValidationPlan>,
}

impl EligibleResolverAsset {
    pub fn has_artifact_validation_plan(&self) -> bool {
        self.artifact_validation_plan.is_some()
    }

    pub fn artifact_validation_plan(&self) -> Option<&ArtifactValidationPlan> {
        self.artifact_validation_plan.as_ref()
    }

    /// Test seam: build an eligible asset around a frozen access without an
    /// artifact validation plan, for callers that only exercise the policy's
    /// refresh kind.
    #[cfg(test)]
    pub(crate) fn for_test_without_validation_plan(
        frozen_access: FrozenAssetAccess,
        policy: &'static AssetExecutionPolicyDefinition,
    ) -> Self {
        Self {
            frozen_access,
            policy,
            artifact_validation_plan: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ResolverCutoverEligibility {
    Eligible(Box<EligibleResolverAsset>),
    Ineligible(EligibilityReason),
    Invalid(ExecutionPolicyError),
}

fn metadata_text<'a>(item: &'a SnapshotItem, key: &'static str) -> Option<&'a str> {
    item.metadata.get(key).and_then(|value| value.as_str())
}

fn safe_basename(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\0')
        && !value.contains('/')
        && !value.contains('\\')
        && value != "."
        && value != ".."
}

fn invalid(reason: ExecutionPolicyError) -> ResolverCutoverEligibility {
    ResolverCutoverEligibility::Invalid(reason)
}

pub fn evaluate_resolver_cutover_eligibility(
    item: &SnapshotItem,
    profiles: &AsfCollectionProfileRegistry,
    policies: &AssetExecutionPolicyRegistry,
) -> ResolverCutoverEligibility {
    evaluate_resolver_cutover_eligibility_with_artifact_validators(
        item,
        profiles,
        policies,
        &ArtifactValidatorRegistry::standard(),
    )
}

pub(crate) fn evaluate_resolver_cutover_eligibility_with_artifact_validators(
    item: &SnapshotItem,
    profiles: &AsfCollectionProfileRegistry,
    policies: &AssetExecutionPolicyRegistry,
    artifact_validators: &ArtifactValidatorRegistry,
) -> ResolverCutoverEligibility {
    if let Err(reason) = item.validate() {
        return invalid(ExecutionPolicyError::Structural(reason));
    }
    let policy = match validate_execution_policy(item.execution_policy.as_ref(), profiles, policies)
    {
        Ok(ValidatedExecutionPolicy::ResolverAuthoritative(policy)) => policy,
        Ok(_) => {
            return ResolverCutoverEligibility::Ineligible(EligibilityReason::ExecutionAuthority)
        }
        Err(reason) => return invalid(reason),
    };
    match policy.delivery_model {
        ExecutionDeliveryModel::Asf(AsfDeliveryModel::MultiAssetProduct)
            if policy.policy_id == ASF_BURST_ASSET_EXECUTION_POLICY_ID =>
        {
            evaluate_burst_resolver_cutover_eligibility(item, policy)
        }
        ExecutionDeliveryModel::Asf(_) => {
            evaluate_asf_resolver_cutover_eligibility(item, profiles, policy)
        }
        ExecutionDeliveryModel::CdseArchiveFirst => {
            evaluate_cdse_resolver_cutover_eligibility(item, policy)
        }
        ExecutionDeliveryModel::UsgsM2mLandsatBundle => {
            evaluate_landsat_resolver_cutover_eligibility(item, policy)
        }
        ExecutionDeliveryModel::PlanetaryComputerLandsatAssets => {
            evaluate_planetary_computer_landsat_eligibility(item, policy)
        }
        ExecutionDeliveryModel::PlanetaryComputerHlsAssets => {
            evaluate_planetary_computer_hls_eligibility(item, policy)
        }
        ExecutionDeliveryModel::PlanetaryComputerReviewedCog => {
            evaluate_planetary_computer_reviewed_cog_eligibility(item, policy)
        }
        ExecutionDeliveryModel::EarthSearchSentinel2BandCog => {
            evaluate_earth_search_sentinel2_band_cog_eligibility(item, policy)
        }
        ExecutionDeliveryModel::Alos2ArchiveFirst => {
            evaluate_alos2_resolver_cutover_eligibility(item, policy)
        }
        ExecutionDeliveryModel::NisarArchiveFirst => {
            evaluate_nisar_resolver_cutover_eligibility(item, policy, artifact_validators)
        }
        ExecutionDeliveryModel::NisarSsarArchiveFirst => {
            evaluate_bhoonidhi_resolver_cutover_eligibility(item, policy, artifact_validators)
        }
    }
}

fn evaluate_asf_resolver_cutover_eligibility(
    item: &SnapshotItem,
    profiles: &AsfCollectionProfileRegistry,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let Some(profile) = profiles.get(policy.collection_profile_id) else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    if profile.profile_id != policy.collection_profile_id {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if policy.delivery_model != ExecutionDeliveryModel::Asf(profile.delivery_model) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    if item.provider_id != "asf.catalog" {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "identity_rule_id") != Some(profile.rule_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    let dataset = metadata_text(item, "dataset");
    if dataset != Some(profile.dataset) || dataset != Some("SENTINEL-1") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    let processing_level = metadata_text(item, "processing_level");
    if !processing_level
        .map(|value| policy.approved_processing_levels.contains(&value))
        .unwrap_or(false)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }
    let product_family = metadata_text(item, "product_family");
    let approved_pair = matches!(
        (product_family, processing_level),
        (Some("SLC"), Some("SLC"))
            | (
                Some("GRD"),
                Some("GRD_HD" | "GRD_HS" | "GRD_MD" | "GRD_MS" | "GRD_FD")
            )
    );
    if !approved_pair
        || !product_family
            .map(|value| policy.approved_product_families.contains(&value))
            .unwrap_or(false)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductFamily);
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
        || !profile
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
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
            .map(|value| policy.approved_credential_profiles.contains(&value))
            .unwrap_or(false)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    if access.descriptor.stable_href.is_none()
        || access.expected_size_bytes.is_none()
        || access.checksum_algorithm.is_none()
        || access.checksum_value.is_none()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "complete frozen access facts are required".to_string(),
        ));
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }

    let reference = &access.reference;
    let provider_product_id = metadata_text(item, "provider_product_id");
    let provider_product_key = metadata_text(item, "provider_product_key");
    let canonical_product_key = metadata_text(item, "canonical_product_key");
    if provider_product_id.is_none() {
        return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id"));
    }
    if provider_product_key.is_none() {
        return invalid(ExecutionPolicyError::MissingIdentity(
            "provider_product_key",
        ));
    }
    if reference.provider_id != item.provider_id
        || Some(reference.collection_id.as_str()) != dataset
        || Some(reference.provider_product_id.as_str()) != provider_product_id
        || Some(reference.provider_product_key.as_str()) != provider_product_key
        || item.item_id != reference.provider_product_key
        || reference.canonical_product_key.as_deref() != canonical_product_key
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

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_cdse_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let (expected_dataset, expected_collection, expected_profile_id, expected_identity_rule_id) =
        match policy.policy_id {
            CDSE_SENTINEL2_L2A_POLICY_ID => (
                "optical.sentinel-2.l2a",
                "SENTINEL-2",
                CDSE_SENTINEL2_L2A_PROFILE_ID,
                CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID,
            ),
            CDSE_SENTINEL2_L1C_POLICY_ID => (
                "optical.sentinel-2.l1c",
                "SENTINEL-2",
                CDSE_SENTINEL2_L1C_PROFILE_ID,
                CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID,
            ),
            CDSE_SENTINEL3_STANDARD_POLICY_ID => (
                "optical.sentinel-3.standard",
                "SENTINEL-3",
                CDSE_SENTINEL3_STANDARD_PROFILE_ID,
                CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID,
            ),
            _ => return invalid(ExecutionPolicyError::ResolverProfileMismatch),
        };
    if policy.collection_profile_id != expected_profile_id
        || policy.delivery_model != ExecutionDeliveryModel::CdseArchiveFirst
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::CdseOAuthBearerV1
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != CDSE_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some(expected_dataset) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(expected_collection) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(expected_profile_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(expected_identity_rule_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    let properties = match item.metadata.get("properties").and_then(Value::as_object) {
        Some(properties) => properties,
        None => {
            return invalid(ExecutionPolicyError::Structural(
                "properties missing".to_string(),
            ))
        }
    };
    let Some(product_type) = properties.get("product_type").and_then(Value::as_str) else {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    };
    if !policy.approved_product_families.contains(&product_type) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    }
    if properties.get("online").and_then(Value::as_bool) != Some(true) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Online);
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
    if access.descriptor.scheme != AssetAccessScheme::OauthBearerHttps
        || access.descriptor.resolver_id != CDSE_ARCHIVE_RESOLVER_ID
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if !access.descriptor.requires_credentials
        || access.descriptor.credential_profile.as_deref() != Some(CDSE_CREDENTIAL_PROFILE)
        || !policy
            .approved_credential_profiles
            .contains(&CDSE_CREDENTIAL_PROFILE)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_cdse_archive_basename(file_name) || file_name != format!("{}.zip", item.display_name) {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0) {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive frozen size is required".to_string(),
        ));
    }
    if !valid_cdse_checksum(
        access.checksum_algorithm.as_deref(),
        access.checksum_value.as_deref(),
    ) {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "valid frozen checksum is required".to_string(),
        ));
    }

    let reference = &access.reference;
    let provider_product_id = match metadata_text(item, "provider_product_id") {
        Some(value) if canonical_cdse_odata_uuid(value) => value,
        _ => return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id")),
    };
    let provider_product_key = match metadata_text(item, "provider_product_key") {
        Some(value) => value,
        None => {
            return invalid(ExecutionPolicyError::MissingIdentity(
                "provider_product_key",
            ))
        }
    };
    let canonical_product_key = metadata_text(item, "canonical_product_key");
    if reference.provider_id != CDSE_PROVIDER_ID
        || reference.collection_id != expected_collection
        || reference.asset_key != CDSE_ARCHIVE_ASSET_KEY
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != provider_product_key
        || item.item_id != reference.provider_product_key
        || reference.canonical_product_key.as_deref() != canonical_product_key
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, CDSE_ARCHIVE_ASSET_KEY) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    if reference.stable_asset_key != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    let expected_href = stable_cdse_archive_href(provider_product_id);
    if access.descriptor.stable_href.as_deref() != expected_href.as_deref() {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "CDSE stable href does not match the frozen UUID".to_string(),
        ));
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_landsat_resolver_cutover_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let Some(profile) = CollectionProfileRegistry.get(LANDSAT_C2_L2_SCENE_PROFILE_ID) else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    let is_v1 = policy.policy_id == USGS_LANDSAT_C2_L2_POLICY_ID_V1;
    let is_v2 = policy.policy_id == USGS_LANDSAT_C2_L2_POLICY_ID_V2;
    let expected_resolver_id = if is_v1 {
        USGS_LANDSAT_C2_L2_RESOLVER_ID_V1
    } else if is_v2 {
        USGS_LANDSAT_C2_L2_RESOLVER_ID_V2
    } else {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    };
    if policy.collection_profile_id != LANDSAT_C2_L2_SCENE_PROFILE_ID
        || policy.delivery_model != ExecutionDeliveryModel::UsgsM2mLandsatBundle
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::UsgsM2mSignedUrlV1
        || profile.resolver_policy_id != USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != LANDSAT_C2_L2_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some(LANDSAT_C2_L2_LOGICAL_DATASET) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(LANDSAT_C2_L2_NATIVE_COLLECTION_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(LANDSAT_C2_L2_SCENE_PROFILE_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some("usgs.eros.m2m.landsat.c2.l2.identity.v1") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    if metadata_text(item, "source_kind") != Some("landsat")
        || metadata_text(item, "product_kind") != Some("optical_scene")
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductType);
    }

    let properties = match item.metadata.get("properties").and_then(Value::as_object) {
        Some(properties) => properties,
        None => {
            return invalid(ExecutionPolicyError::Structural(
                "properties missing".to_owned(),
            ))
        }
    };
    let processing_level = properties.get("processing_level").and_then(Value::as_str);
    if !processing_level.is_some_and(|value| policy.approved_processing_levels.contains(&value)) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
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
    if access.descriptor.scheme != AssetAccessScheme::UsgsM2m {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AccessScheme);
    }
    if access.descriptor.resolver_id != expected_resolver_id
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if !access.descriptor.requires_credentials
        || access.descriptor.credential_profile.as_deref() != Some(USGS_M2M_SESSION_PROFILE_ID)
        || !policy
            .approved_credential_profiles
            .contains(&USGS_M2M_SESSION_PROFILE_ID)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }

    let provider_product_id = match metadata_text(item, "provider_product_id") {
        Some(value) => value,
        None => return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id")),
    };
    let provider_product_key = match metadata_text(item, "provider_product_key") {
        Some(value) => value,
        None => {
            return invalid(ExecutionPolicyError::MissingIdentity(
                "provider_product_key",
            ))
        }
    };
    if properties.get("entity_id").and_then(Value::as_str) != Some(provider_product_id)
        || properties.get("landsat_product_id").and_then(Value::as_str)
            != Some(item.display_name.as_str())
        || properties.get("dataset_alias").and_then(Value::as_str)
            != Some(LANDSAT_C2_L2_NATIVE_COLLECTION_ID)
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("landsat_properties"));
    }

    let reference = &access.reference;
    if reference.provider_id != LANDSAT_C2_L2_PROVIDER_ID
        || reference.collection_id != LANDSAT_C2_L2_NATIVE_COLLECTION_ID
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != provider_product_key
        || item.item_id != reference.provider_product_key
        || reference.canonical_product_key.as_deref()
            != metadata_text(item, "canonical_product_key")
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

    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) || file_name != format!("{}.tar", item.display_name) {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }
    if access.expected_size_bytes.is_some_and(|value| value == 0) {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive frozen size is required when present".to_owned(),
        ));
    }
    let checksum_valid = match (
        access.checksum_algorithm.as_deref(),
        access.checksum_value.as_deref(),
    ) {
        (Some(algorithm), Some(value)) => crate::ExpectedChecksum::new(algorithm, value).is_ok(),
        (None, None) => false,
        _ => {
            return invalid(ExecutionPolicyError::InvalidAssetAccess(
                "checksum algorithm and value must be paired".to_owned(),
            ))
        }
    };
    if access.expected_size_bytes.is_none() && !checksum_valid {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "frozen size or supported checksum authority is required".to_owned(),
        ));
    }

    let locator = match access.descriptor.stable_href.as_deref().and_then(|value| {
        parse_frozen_landsat_bundle_locator_for_resolver(value, expected_resolver_id)
    }) {
        Some(locator) => locator,
        None => {
            return invalid(ExecutionPolicyError::InvalidAssetAccess(
                "USGS Landsat stable href is not the exact bundle identity".to_owned(),
            ))
        }
    };
    if locator.dataset_alias != reference.collection_id
        || locator.entity_id != reference.provider_product_id
        || locator.option_id != reference.asset_key
        || locator.display_id != item.display_name
    {
        return invalid(ExecutionPolicyError::IdentityMismatch(
            "landsat_bundle_locator",
        ));
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_planetary_computer_landsat_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let profile_id = match policy.policy_id {
        PLANETARY_COMPUTER_LANDSAT_C2_L2_POLICY_ID => LANDSAT_C2_L2_SCENE_PROFILE_ID,
        MPC_LANDSAT_ASSET_POLICY_ID => MPC_LANDSAT_SCENE_PROFILE_ID,
        _ => return invalid(ExecutionPolicyError::ResolverProfileMismatch),
    };
    let profile = CollectionProfileRegistry
        .get(profile_id)
        .expect("both reviewed Landsat source profiles exist");
    if policy.collection_profile_id != profile.profile_id
        || policy.delivery_model != ExecutionDeliveryModel::PlanetaryComputerLandsatAssets
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::PlanetaryComputerSasV1
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != profile.provider_id {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some(LANDSAT_C2_L2_LOGICAL_DATASET) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(profile.native_collection_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(profile.profile_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(profile.identity_rule_id) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first")
        || metadata_text(item, "source_kind") != Some("landsat")
        || metadata_text(item, "product_kind") != Some("optical_scene")
        || !item.payload.is_empty()
    {
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
    let processing_level = properties.get("processing_level").and_then(Value::as_str);
    if !processing_level.is_some_and(|value| policy.approved_processing_levels.contains(&value)) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }
    let source_display_id = match properties.get("landsat_product_id").and_then(Value::as_str) {
        Some(value) => value,
        None => return invalid(ExecutionPolicyError::MissingIdentity("landsat_product_id")),
    };
    let provider_product_id = match metadata_text(item, "provider_product_id") {
        Some(value) => value,
        None => return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id")),
    };
    let provider_product_key = match metadata_text(item, "provider_product_key") {
        Some(value) => value,
        None => {
            return invalid(ExecutionPolicyError::MissingIdentity(
                "provider_product_key",
            ))
        }
    };
    if properties.get("entity_id").and_then(Value::as_str) != Some(provider_product_id)
        || properties.get("dataset_alias").and_then(Value::as_str)
            != Some(profile.native_collection_id)
        || metadata_text(item, "source_item_id") != Some(provider_product_key)
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("landsat_properties"));
    }

    if profile.profile_id == MPC_LANDSAT_SCENE_PROFILE_ID {
        if insar_provider::planetary_computer::validate_planetary_computer_landsat_source_id(
            provider_product_id,
            source_display_id,
        )
        .is_err()
        {
            return invalid(ExecutionPolicyError::IdentityMismatch("microsoft_item_id"));
        }
        // Consistently duplicated metadata is not independent identity proof.
        let expected_key = insar_core::provider_product_key(
            profile.provider_id,
            profile.native_collection_id,
            provider_product_id,
        );
        if expected_key.as_deref() != Ok(provider_product_key) {
            return invalid(ExecutionPolicyError::IdentityMismatch(
                "microsoft_product_key",
            ));
        }
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
    if access.descriptor.scheme != AssetAccessScheme::PublicHttps {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AccessScheme);
    }
    if access.descriptor.resolver_id != PLANETARY_COMPUTER_LANDSAT_RESOLVER_ID
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if access.descriptor.requires_credentials
        || access.descriptor.credential_profile.is_some()
        || !policy.approved_credential_profiles.is_empty()
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0)
        || access.checksum_algorithm.is_some()
        || access.checksum_value.is_some()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive frozen size without persisted signed material is required".to_owned(),
        ));
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) || item.display_name != file_name {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }

    let reference = &access.reference;
    if reference.provider_id != profile.provider_id
        || reference.collection_id != profile.native_collection_id
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != provider_product_key
        || reference.canonical_product_key.as_deref()
            != metadata_text(item, "canonical_product_key")
        || metadata_text(item, "delivery_asset_key") != Some(reference.asset_key.as_str())
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, &reference.asset_key) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    if reference.stable_asset_key != expected_stable_key || item.item_id != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "unsigned stable asset href is required".to_owned(),
        ));
    };
    if validate_planetary_computer_landsat_asset(
        stable_href,
        source_display_id,
        &reference.asset_key,
        file_name,
        &access.role,
    )
    .is_err()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "Planetary Computer Landsat asset identity is invalid".to_owned(),
        ));
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_planetary_computer_reviewed_cog_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let (class, expected_resolver, source_kind, product_kind, product_family, processing_level) =
        match policy.policy_id {
            PLANETARY_COMPUTER_SENTINEL1_GRD_COG_POLICY_ID => (
                PlanetaryComputerReviewedAssetClass::Sentinel1GrdCog,
                PLANETARY_COMPUTER_SENTINEL1_GRD_RESOLVER_ID,
                "sentinel1",
                "sar_grd",
                "SENTINEL1_GRD_COG",
                "GRD",
            ),
            PLANETARY_COMPUTER_COPDEM_GLO30_COG_POLICY_ID => (
                PlanetaryComputerReviewedAssetClass::CopDemGlo30Cog,
                PLANETARY_COMPUTER_COPDEM_GLO30_RESOLVER_ID,
                "dem",
                "dem_raster",
                "COPDEM_GLO30_COG",
                "GLO-30",
            ),
            PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_COG_POLICY_ID => (
                PlanetaryComputerReviewedAssetClass::Sentinel2L2aBandCog,
                PLANETARY_COMPUTER_SENTINEL2_L2A_BAND_RESOLVER_ID,
                "sentinel2",
                "optical_scene",
                "SENTINEL2_L2A_BAND_COG",
                "L2A",
            ),
            _ => return invalid(ExecutionPolicyError::ResolverProfileMismatch),
        };
    if policy.collection_profile_id != class.collection_profile_id()
        || policy.delivery_model != ExecutionDeliveryModel::PlanetaryComputerReviewedCog
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::PlanetaryComputerSasV1
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some(class.logical_dataset()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(class.collection_id()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(class.collection_profile_id()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(MPC_REVIEWED_STAC_IDENTITY_RULE_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("reviewed_cog")
        || metadata_text(item, "source_kind") != Some(source_kind)
        || metadata_text(item, "product_kind") != Some(product_kind)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    if metadata_text(item, "product_family") != Some(product_family)
        || !policy.approved_product_families.contains(&product_family)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductFamily);
    }
    if metadata_text(item, "processing_level") != Some(processing_level)
        || !policy
            .approved_processing_levels
            .contains(&processing_level)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }

    let provider_product_id = match metadata_text(item, "provider_product_id") {
        Some(value) => value,
        None => return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id")),
    };
    let expected_product_key = match insar_core::provider_product_key(
        PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
        class.collection_id(),
        provider_product_id,
    ) {
        Ok(value) => value,
        Err(_) => {
            return invalid(ExecutionPolicyError::IdentityMismatch(
                "provider_product_key",
            ))
        }
    };
    if metadata_text(item, "provider_product_key") != Some(expected_product_key.as_str()) {
        return invalid(ExecutionPolicyError::IdentityMismatch(
            "provider_product_key",
        ));
    }
    if metadata_text(item, "source_item_id").is_none_or(|value| value.trim().is_empty()) {
        return invalid(ExecutionPolicyError::MissingIdentity("source_item_id"));
    }

    let Some(access) = item.asset_access.as_ref() else {
        return invalid(ExecutionPolicyError::MissingAssetAccess);
    };
    if let Err(reason) = access.validate() {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(reason));
    }
    if access.role != AssetRole::Primary || !policy.approved_asset_roles.contains(&access.role) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AssetRole);
    }
    if access.descriptor.scheme != AssetAccessScheme::PublicHttps {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AccessScheme);
    }
    if access.descriptor.resolver_id != expected_resolver
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if access.descriptor.requires_credentials
        || access.descriptor.credential_profile.is_some()
        || !policy.approved_credential_profiles.is_empty()
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0)
        || access.checksum_algorithm.is_some()
        || access.checksum_value.is_some()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive frozen size without persisted signed material is required".to_owned(),
        ));
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) || item.display_name != file_name {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }

    let reference = &access.reference;
    if reference.provider_id != PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID
        || reference.collection_id != class.collection_id()
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != expected_product_key
        || reference.canonical_product_key.is_some()
        || metadata_text(item, "delivery_asset_key") != Some(reference.asset_key.as_str())
        || metadata_text(item, "delivery_asset_role") != Some("primary")
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, &reference.asset_key) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    if reference.stable_asset_key != expected_stable_key || item.item_id != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "unsigned stable asset href is required".to_owned(),
        ));
    };
    match validate_planetary_computer_reviewed_asset_identity(
        class,
        provider_product_id,
        &reference.asset_key,
        stable_href,
    ) {
        Ok(expected_name) if expected_name == file_name => {}
        _ => {
            return invalid(ExecutionPolicyError::InvalidAssetAccess(
                "Planetary Computer reviewed COG identity is invalid".to_owned(),
            ))
        }
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_earth_search_sentinel2_band_cog_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    if policy.policy_id != EARTH_SEARCH_SENTINEL2_L2A_BAND_COG_POLICY_ID
        || policy.collection_profile_id != EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID
        || policy.delivery_model != ExecutionDeliveryModel::EarthSearchSentinel2BandCog
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::PublicAnonymousHttpsV1
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some("optical.sentinel-2.l2a") {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id")
        != Some(EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID)
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("reviewed_cog")
        || metadata_text(item, "source_kind") != Some("sentinel2")
        || metadata_text(item, "product_kind") != Some("optical_scene")
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }
    if metadata_text(item, "product_family") != Some("SENTINEL2_L2A_BAND_COG")
        || !policy
            .approved_product_families
            .contains(&"SENTINEL2_L2A_BAND_COG")
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProductFamily);
    }
    if metadata_text(item, "processing_level") != Some("L2A")
        || !policy.approved_processing_levels.contains(&"L2A")
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel);
    }

    let provider_product_id = match metadata_text(item, "provider_product_id") {
        Some(value) => value,
        None => return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id")),
    };
    let expected_product_key = match insar_core::provider_product_key(
        EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
        EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
        provider_product_id,
    ) {
        Ok(value) => value,
        Err(_) => {
            return invalid(ExecutionPolicyError::IdentityMismatch(
                "provider_product_key",
            ))
        }
    };
    if metadata_text(item, "provider_product_key") != Some(expected_product_key.as_str()) {
        return invalid(ExecutionPolicyError::IdentityMismatch(
            "provider_product_key",
        ));
    }
    if metadata_text(item, "source_item_id").is_none_or(|value| value.trim().is_empty()) {
        return invalid(ExecutionPolicyError::MissingIdentity("source_item_id"));
    }

    let Some(access) = item.asset_access.as_ref() else {
        return invalid(ExecutionPolicyError::MissingAssetAccess);
    };
    if let Err(reason) = access.validate() {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(reason));
    }
    if access.role != AssetRole::Primary || !policy.approved_asset_roles.contains(&access.role) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AssetRole);
    }
    if access.descriptor.scheme != AssetAccessScheme::PublicHttps {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AccessScheme);
    }
    if access.descriptor.resolver_id != EARTH_SEARCH_SENTINEL2_L2A_BAND_RESOLVER_ID
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if access.descriptor.requires_credentials
        || access.descriptor.credential_profile.is_some()
        || !policy.approved_credential_profiles.is_empty()
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    if access.expected_size_bytes.is_none_or(|size| size == 0)
        || access.checksum_algorithm.is_some()
        || access.checksum_value.is_some()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "positive frozen size without persisted signed material is required".to_owned(),
        ));
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    if !safe_basename(file_name) || item.display_name != file_name {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }

    let reference = &access.reference;
    if reference.provider_id != EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
        || reference.collection_id != EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != expected_product_key
        || reference.canonical_product_key.is_some()
        || metadata_text(item, "delivery_asset_key") != Some(reference.asset_key.as_str())
        || metadata_text(item, "delivery_asset_role") != Some("primary")
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, &reference.asset_key) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    if reference.stable_asset_key != expected_stable_key || item.item_id != expected_stable_key {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "unsigned stable asset href is required".to_owned(),
        ));
    };
    match validate_earth_search_sentinel2_stable_band_url(
        provider_product_id,
        &reference.asset_key,
        stable_href,
    ) {
        Ok(expected_name) if expected_name == file_name => {}
        _ => {
            return invalid(ExecutionPolicyError::InvalidAssetAccess(
                "Earth Search Sentinel-2 band COG identity is invalid".to_owned(),
            ))
        }
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

fn evaluate_planetary_computer_hls_eligibility(
    item: &SnapshotItem,
    policy: &'static AssetExecutionPolicyDefinition,
) -> ResolverCutoverEligibility {
    let (product, expected_source_kind, supported_bands): (
        MpcHlsProduct,
        &'static str,
        &'static [&'static str],
    ) = match policy.policy_id {
        PLANETARY_COMPUTER_HLS2_L30_POLICY_ID => (
            MpcHlsProduct::L30,
            "landsat",
            &[
                "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B09", "B10", "B11", "Fmask",
            ],
        ),
        PLANETARY_COMPUTER_HLS2_S30_POLICY_ID => (
            MpcHlsProduct::S30,
            "sentinel2",
            &[
                "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11",
                "B12", "Fmask",
            ],
        ),
        _ => return invalid(ExecutionPolicyError::ResolverProfileMismatch),
    };
    if policy.collection_profile_id != product.profile_id()
        || policy.delivery_model != ExecutionDeliveryModel::PlanetaryComputerHlsAssets
        || policy.auth_refresh_policy_id != AuthRefreshPolicyId::PlanetaryComputerSasV1
    {
        return invalid(ExecutionPolicyError::ResolverProfileMismatch);
    }
    if item.provider_id != MPC_HLS_PROVIDER_ID {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Provider);
    }
    if metadata_text(item, "logical_dataset") != Some(product.logical_dataset()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::Dataset);
    }
    if metadata_text(item, "native_collection_id") != Some(product.collection_id()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::NativeCollection);
    }
    if metadata_text(item, "collection_profile_id") != Some(product.profile_id()) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CollectionProfile);
    }
    if metadata_text(item, "identity_rule_id") != Some(MPC_HLS_IDENTITY_RULE_ID) {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::IdentityRule);
    }
    if metadata_text(item, "delivery_model") != Some("archive_first")
        || metadata_text(item, "source_kind") != Some(expected_source_kind)
        || metadata_text(item, "product_kind") != Some("optical_scene")
        || !item.payload.is_empty()
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::DeliveryModel);
    }

    let Some(provider_product_id) = metadata_text(item, "provider_product_id") else {
        return invalid(ExecutionPolicyError::MissingIdentity("provider_product_id"));
    };
    if !valid_mpc_hls_scene_id(product, provider_product_id) {
        return invalid(ExecutionPolicyError::IdentityMismatch("hls_scene_id"));
    }
    let expected_product_key = match insar_core::provider_product_key(
        MPC_HLS_PROVIDER_ID,
        product.collection_id(),
        provider_product_id,
    ) {
        Ok(value) => value,
        Err(_) => {
            return invalid(ExecutionPolicyError::IdentityMismatch(
                "provider_product_key",
            ))
        }
    };
    if metadata_text(item, "provider_product_key") != Some(expected_product_key.as_str()) {
        return invalid(ExecutionPolicyError::IdentityMismatch(
            "provider_product_key",
        ));
    }
    if metadata_text(item, "source_item_id") != Some(expected_product_key.as_str()) {
        return invalid(ExecutionPolicyError::MissingIdentity("source_item_id"));
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
    if access.descriptor.scheme != AssetAccessScheme::PublicHttps {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::AccessScheme);
    }
    if access.descriptor.resolver_id != PLANETARY_COMPUTER_HLS_RESOLVER_ID
        || !policy
            .approved_resolver_ids
            .contains(&access.descriptor.resolver_id.as_str())
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::ResolverId);
    }
    if access.descriptor.requires_credentials
        || access.descriptor.credential_profile.is_some()
        || !policy.approved_credential_profiles.is_empty()
    {
        return ResolverCutoverEligibility::Ineligible(EligibilityReason::CredentialProfile);
    }
    // Live MPC HLS STAC assets omit `file:size`, so `None` is valid while `Some(0)` is rejected.
    if access.expected_size_bytes == Some(0)
        || access.checksum_algorithm.is_some()
        || access.checksum_value.is_some()
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "non-zero optional size without persisted signed material is required".to_owned(),
        ));
    }

    let reference = &access.reference;
    if reference.provider_id != MPC_HLS_PROVIDER_ID
        || reference.collection_id != product.collection_id()
        || reference.provider_product_id != provider_product_id
        || reference.provider_product_key != expected_product_key
        || reference.canonical_product_key.is_some()
        || !supported_bands.contains(&reference.asset_key.as_str())
        || metadata_text(item, "delivery_asset_key") != Some(reference.asset_key.as_str())
    {
        return invalid(ExecutionPolicyError::IdentityMismatch("stable_reference"));
    }
    let Some(file_name) = access.file_name.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidFilename);
    };
    let expected_tif = format!("{provider_product_id}.{}.tif", reference.asset_key);
    let expected_tiff = format!("{provider_product_id}.{}.tiff", reference.asset_key);
    if !safe_basename(file_name)
        || item.display_name != file_name
        || (file_name != expected_tif && file_name != expected_tiff)
    {
        return invalid(ExecutionPolicyError::InvalidFilename);
    }
    let expected_stable_key =
        match stable_asset_key(&reference.provider_product_key, &reference.asset_key) {
            Ok(value) => value,
            Err(_) => return invalid(ExecutionPolicyError::InvalidStableAssetKey),
        };
    let expected_band_item_id = format!(
        "{expected_product_key}-{}",
        reference.asset_key.to_ascii_lowercase()
    );
    if reference.stable_asset_key != expected_stable_key
        || (item.item_id != expected_band_item_id && item.item_id != expected_stable_key)
    {
        return invalid(ExecutionPolicyError::InvalidStableAssetKey);
    }
    let Some(stable_href) = access.descriptor.stable_href.as_deref() else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "unsigned stable asset href is required".to_owned(),
        ));
    };
    let Ok(parsed_url) = reqwest::Url::parse(stable_href) else {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "Planetary Computer HLS asset href is invalid".to_owned(),
        ));
    };
    if !is_valid_planetary_computer_hls_href(stable_href)
        || parsed_url.query().is_some()
        || !parsed_url
            .path()
            .ends_with(&format!("/{provider_product_id}/{file_name}"))
    {
        return invalid(ExecutionPolicyError::InvalidAssetAccess(
            "Planetary Computer HLS asset identity is invalid".to_owned(),
        ));
    }

    ResolverCutoverEligibility::Eligible(Box::new(EligibleResolverAsset {
        frozen_access: access.clone(),
        policy,
        artifact_validation_plan: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::{
        provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
        AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, FrozenAssetAccess,
        StableAssetReference,
    };
    use serde_json::{json, Map};

    fn sample_hls_band_item(
        product: MpcHlsProduct,
        band: &str,
        expected_size_bytes: Option<u64>,
        query_suffix: &str,
    ) -> SnapshotItem {
        let product_id = match product {
            MpcHlsProduct::L30 => "HLS.L30.T49RDP.2024177T030756.v2.0",
            MpcHlsProduct::S30 => "HLS.S30.T51SUA.2025165T024529.v2.0",
        };
        let source_kind = match product {
            MpcHlsProduct::L30 => "landsat",
            MpcHlsProduct::S30 => "sentinel2",
        };
        let pkey =
            provider_product_key(MPC_HLS_PROVIDER_ID, product.collection_id(), product_id).unwrap();
        let skey = stable_asset_key(&pkey, band).unwrap();
        let file_name = format!("{product_id}.{band}.tif");
        let stable_href = format!(
            "https://hls2euwest.blob.core.windows.net/hls2/{}/51/S/UA/2025/06/14/{product_id}/{file_name}{query_suffix}",
            product.product_token()
        );
        SnapshotItem {
            item_id: format!("{pkey}-{}", band.to_ascii_lowercase()),
            provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
            display_name: file_name.clone(),
            acquisition_datetime: Some("2025-06-14T02:57:32.351130Z".to_owned()),
            footprint_geojson: None,
            metadata: Map::from_iter([
                (
                    "logical_dataset".to_owned(),
                    json!(product.logical_dataset()),
                ),
                (
                    "native_collection_id".to_owned(),
                    json!(product.collection_id()),
                ),
                (
                    "collection_profile_id".to_owned(),
                    json!(product.profile_id()),
                ),
                (
                    "identity_rule_id".to_owned(),
                    json!(MPC_HLS_IDENTITY_RULE_ID),
                ),
                ("delivery_model".to_owned(), json!("archive_first")),
                ("source_kind".to_owned(), json!(source_kind)),
                ("product_kind".to_owned(), json!("optical_scene")),
                ("provider_product_id".to_owned(), json!(product_id)),
                ("provider_product_key".to_owned(), json!(pkey)),
                ("source_item_id".to_owned(), json!(pkey)),
                ("delivery_asset_key".to_owned(), json!(band)),
                ("delivery_asset_role".to_owned(), json!("primary")),
                ("delivery_product_directory".to_owned(), json!(product_id)),
            ]),
            payload: Map::new(),
            asset_access: Some(FrozenAssetAccess {
                descriptor: AssetAccessDescriptor {
                    resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID.to_owned(),
                    scheme: AssetAccessScheme::PublicHttps,
                    requires_credentials: false,
                    credential_profile: None,
                    stable_href: Some(stable_href),
                    resolver_parameters: AssetResolverParameters {},
                },
                reference: StableAssetReference {
                    provider_id: MPC_HLS_PROVIDER_ID.to_owned(),
                    collection_id: product.collection_id().to_owned(),
                    provider_product_id: product_id.to_owned(),
                    provider_product_key: pkey,
                    asset_key: band.to_owned(),
                    stable_asset_key: skey,
                    canonical_product_key: None,
                },
                role: AssetRole::Primary,
                file_name: Some(file_name),
                expected_size_bytes,
                checksum_algorithm: None,
                checksum_value: None,
            }),
            execution_policy: Some(AssetExecutionPolicy {
                authority: AssetExecutionAuthority::ResolverAuthoritative,
                policy_id: product.asset_policy_id().to_owned(),
                collection_profile_id: Some(product.profile_id().to_owned()),
            }),
        }
    }

    #[test]
    fn planetary_computer_hls_l30_and_s30_items_with_unknown_size_are_eligible() {
        let profiles = AsfCollectionProfileRegistry;
        let policies = AssetExecutionPolicyRegistry;
        for (product, band) in [
            (MpcHlsProduct::L30, "B02"),
            (MpcHlsProduct::L30, "Fmask"),
            (MpcHlsProduct::S30, "B8A"),
            (MpcHlsProduct::S30, "Fmask"),
        ] {
            let item = sample_hls_band_item(product, band, None, "");
            let verdict = evaluate_resolver_cutover_eligibility(&item, &profiles, &policies);
            assert!(
                matches!(verdict, ResolverCutoverEligibility::Eligible(_)),
                "expected {:?} {band} eligible, got {verdict:?}",
                product
            );
        }
    }

    #[test]
    fn planetary_computer_hls_rejects_persisted_sas_query_zero_size_and_profile_mismatch() {
        let profiles = AsfCollectionProfileRegistry;
        let policies = AssetExecutionPolicyRegistry;

        let signed = sample_hls_band_item(MpcHlsProduct::S30, "B01", None, "?sig=secret");
        assert!(matches!(
            evaluate_resolver_cutover_eligibility(&signed, &profiles, &policies),
            ResolverCutoverEligibility::Invalid(ExecutionPolicyError::Structural(_))
        ));

        let mut bad_host = sample_hls_band_item(MpcHlsProduct::S30, "B01", None, "");
        bad_host
            .asset_access
            .as_mut()
            .unwrap()
            .descriptor
            .stable_href = Some(
            "https://evil.example.com/hls2/S30/51/S/UA/2025/06/14/HLS.S30.T51SUA.2025165T024529.v2.0/HLS.S30.T51SUA.2025165T024529.v2.0.B01.tif".to_owned(),
        );
        assert!(matches!(
            evaluate_resolver_cutover_eligibility(&bad_host, &profiles, &policies),
            ResolverCutoverEligibility::Invalid(ExecutionPolicyError::InvalidAssetAccess(_))
        ));

        let zero_size = sample_hls_band_item(MpcHlsProduct::S30, "B01", Some(0), "");
        assert!(matches!(
            evaluate_resolver_cutover_eligibility(&zero_size, &profiles, &policies),
            ResolverCutoverEligibility::Invalid(
                ExecutionPolicyError::Structural(_) | ExecutionPolicyError::InvalidAssetAccess(_)
            )
        ));

        let mut mismatch = sample_hls_band_item(MpcHlsProduct::L30, "B02", None, "");
        mismatch.execution_policy.as_mut().unwrap().policy_id =
            PLANETARY_COMPUTER_HLS2_S30_POLICY_ID.to_owned();
        assert!(matches!(
            evaluate_resolver_cutover_eligibility(&mismatch, &profiles, &policies),
            ResolverCutoverEligibility::Invalid(ExecutionPolicyError::ResolverProfileMismatch)
        ));
    }
}
