use insar_core::AssetRole;

pub use insar_core::CatalogDeliveryModel;

use crate::planetary_computer_asset::{
    MPC_COPDEM_GLO30_ASSET_POLICY_ID, MPC_COPDEM_GLO30_COLLECTION_ID,
    MPC_COPDEM_GLO30_FILENAME_RULE_ID, MPC_COPDEM_GLO30_LOGICAL_DATASET,
    MPC_COPDEM_GLO30_PROFILE_ID, MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
    MPC_SENTINEL1_GRD_ASSET_POLICY_ID, MPC_SENTINEL1_GRD_COLLECTION_ID,
    MPC_SENTINEL1_GRD_FILENAME_RULE_ID, MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    MPC_SENTINEL1_GRD_PROFILE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use crate::usgs_m2m::landsat_scene_search_live_accepted;

use super::mpc_hls::{
    MPC_HLS_FILENAME_RULE_ID, MPC_HLS_IDENTITY_RULE_ID, MPC_HLS_L30_COLLECTION_ID,
    MPC_HLS_L30_LOGICAL_DATASET, MPC_HLS_L30_PROFILE_ID, MPC_HLS_PROVIDER_ID,
    MPC_HLS_S30_COLLECTION_ID, MPC_HLS_S30_LOGICAL_DATASET, MPC_HLS_S30_PROFILE_ID,
    MPC_HLS_SEARCH_ONLY_POLICY_ID,
};

pub const SENTINEL2_L2A_PROFILE_ID: &str = "cdse.sentinel-2.l2a.collection.v1";
pub const CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID: &str = "cdse.sentinel-2.l2a.identity.v1";
pub const CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-2.l2a.archive-filename.v1";
pub const CDSE_SENTINEL2_L2A_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-2.l2a.archive-resolver-policy.v1";
pub const MPC_SENTINEL2_L2A_PROVIDER_ID: &str = "microsoft.planetary-computer";
pub const MPC_SENTINEL2_L2A_COLLECTION_ID: &str = "sentinel-2-l2a";
pub const MPC_SENTINEL2_L2A_PROFILE_ID: &str = "planetary-computer.sentinel-2-l2a.scene.v1";
pub const MPC_SENTINEL2_L2A_IDENTITY_RULE_ID: &str =
    "planetary-computer.sentinel-2-l2a.identity.v1";
pub const MPC_SENTINEL2_L2A_FILENAME_RULE_ID: &str =
    "planetary-computer.sentinel-2-l2a.product-uri.v1";
pub const MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID: &str =
    "planetary-computer.sentinel-2-l2a.search-only.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID: &str = "element84.earth-search";
pub const EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID: &str = "sentinel-2-l2a";
pub const EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID: &str =
    "element84.earth-search.sentinel-2-l2a.scene.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID: &str =
    "element84.earth-search.sentinel-2-l2a.stac-item-id.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID: &str =
    "element84.earth-search.sentinel-2-l2a.safe-name.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID: &str =
    "element84.earth-search.sentinel-2-l2a.search-only.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_BAND_PROFILE_ID: &str =
    "element84.earth-search.sentinel-2-l2a.band-cog.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_BAND_ASSET_POLICY_ID: &str =
    "element84.earth-search.sentinel-2-l2a.band-cog-resolver-policy.v1";
pub const SENTINEL2_L1C_PROFILE_ID: &str = "cdse.sentinel-2.l1c.collection.v1";
pub const CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID: &str = "cdse.sentinel-2.l1c.identity.v1";
pub const CDSE_SENTINEL2_L1C_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-2.l1c.archive-filename.v1";
pub const CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-2.l1c.archive-resolver-policy.v1";

pub const SENTINEL3_SLSTR_PROFILE_ID: &str = "cdse.sentinel-3.slstr.collection.v1";
pub const CDSE_SENTINEL3_SLSTR_IDENTITY_RULE_ID: &str = "cdse.sentinel-3.slstr.identity.v1";
pub const CDSE_SENTINEL3_SLSTR_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-3.slstr.archive-filename.v1";
pub const CDSE_SENTINEL3_SLSTR_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-3.slstr.archive-resolver-policy.v1";

pub const SENTINEL3_OLCI_PROFILE_ID: &str = "cdse.sentinel-3.olci.collection.v1";
pub const CDSE_SENTINEL3_OLCI_IDENTITY_RULE_ID: &str = "cdse.sentinel-3.olci.identity.v1";
pub const CDSE_SENTINEL3_OLCI_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-3.olci.archive-filename.v1";
pub const CDSE_SENTINEL3_OLCI_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-3.olci.archive-resolver-policy.v1";

pub const SENTINEL3_SRAL_PROFILE_ID: &str = "cdse.sentinel-3.sral.collection.v1";
pub const CDSE_SENTINEL3_SRAL_IDENTITY_RULE_ID: &str = "cdse.sentinel-3.sral.identity.v1";
pub const CDSE_SENTINEL3_SRAL_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-3.sral.archive-filename.v1";
pub const CDSE_SENTINEL3_SRAL_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-3.sral.archive-resolver-policy.v1";

pub const SENTINEL3_STANDARD_PROFILE_ID: &str = "cdse.sentinel-3.standard.collection.v1";
pub const CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID: &str = "cdse.sentinel-3.standard.identity.v1";
pub const CDSE_SENTINEL3_STANDARD_ARCHIVE_FILENAME_RULE_ID: &str =
    "cdse.sentinel-3.standard.archive-filename.v1";
pub const CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID: &str =
    "cdse.sentinel-3.standard.archive-resolver-policy.v1";
pub const LANDSAT_C2_L2_FOUNDATION_PROFILE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.foundation.v1";
pub const LANDSAT_C2_L2_SCENE_PROFILE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.scene.v1";
pub const USGS_LANDSAT_C2_L2_IDENTITY_RULE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.identity.v1";
pub const USGS_LANDSAT_C2_L2_BUNDLE_FILENAME_RULE_ID: &str =
    "usgs.eros.m2m.landsat.c2.l2.full-bundle-filename.v1";
pub const USGS_LANDSAT_C2_L2_LEGACY_SCENE_FILENAME_RULE_ID: &str =
    "usgs.eros.m2m.landsat.c2.l2.filename-deferred.v1";
pub const USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V1: &str =
    "usgs.eros.m2m.landsat.c2.l2.full-bundle-resolver-policy.v1";
pub const USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2: &str =
    "usgs.eros.m2m.landsat.c2.l2.full-bundle-resolver-policy.v2";
pub const USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID: &str =
    USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID_V2;
pub const LANDSAT_C2_L2_LOGICAL_DATASET: &str = "optical.landsat.c2.l2";
pub const LANDSAT_C2_L2_PROVIDER_ID: &str = "usgs.eros.m2m";
pub const LANDSAT_C2_L2_NATIVE_COLLECTION_ID: &str = "landsat_ot_c2_l2";
// Source identity is separate from the shared Landsat scientific product.
// The historical USGS constants remain valid for already-frozen batches/tasks.
pub const MPC_LANDSAT_PROVIDER_ID: &str = "microsoft.planetary-computer";
pub const MPC_LANDSAT_COLLECTION_ID: &str = "landsat-c2-l2";
pub const MPC_LANDSAT_SCENE_PROFILE_ID: &str = "planetary-computer.landsat-c2-l2.scene.v1";
pub const MPC_LANDSAT_IDENTITY_RULE_ID: &str = "planetary-computer.landsat-c2-l2.identity.v1";
pub const MPC_LANDSAT_FILENAME_RULE_ID: &str = "planetary-computer.landsat-c2-l2.asset-filename.v1";
pub const MPC_LANDSAT_ASSET_POLICY_ID: &str = "planetary-computer.landsat-c2-l2.asset-set.v2";
pub const EARTH_SEARCH_LANDSAT_PROVIDER_ID: &str = "element84.earth-search";
pub const EARTH_SEARCH_LANDSAT_COLLECTION_ID: &str = "landsat-c2-l2";
pub const EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID: &str =
    "element84.earth-search.landsat-c2-l2.scene.v1";
pub const EARTH_SEARCH_LANDSAT_IDENTITY_RULE_ID: &str =
    "element84.earth-search.landsat-c2-l2.identity.v1";
pub const EARTH_SEARCH_LANDSAT_FILENAME_RULE_ID: &str =
    "element84.earth-search.landsat-c2-l2.asset-filename.v1";
pub const EARTH_SEARCH_LANDSAT_SEARCH_ONLY_POLICY_ID: &str =
    "element84.earth-search.landsat-c2-l2.search-only.v1";
pub const USGS_M2M_SESSION_PROFILE_ID: &str = crate::usgs_m2m::USGS_M2M_SESSION_PROFILE_ID;
pub const ALOS2_PROFILE_ID: &str = "asf.alos-2.palsar-2.scansar.l1.1.v1";
pub const ALOS2_LOGICAL_DATASET: &str = "sar.alos-2.palsar-2.scansar.l1.1.v1";
pub const ALOS2_PROVIDER_ID: &str = "asf.nasa.earthdata";
pub const ALOS2_NATIVE_COLLECTION_ID: &str = "C3315903479-ASF";
pub const ALOS2_COLLECTION_SHORT_NAME: &str = "ALOS2_L1_PSR2";
pub const ALOS2_COLLECTION_VERSION: &str = "1";
pub const ALOS2_IDENTITY_RULE_ID: &str = "asf.alos-2.cmr-granule.v1";
pub const ALOS2_FILENAME_RULE_ID: &str = "asf.alos-2.scansar-zip.v1";
pub const ALOS2_RESOLVER_POLICY_ID: &str = "asf.alos-2.earthdata-complete-zip.v1";
pub const ALOS2_PROCESSING_LEVELS: &[&str] = &["1.1"];
pub const ALOS2_SCAN_SAR_BEAM_MODES: &[&str] = &["WBS", "WBD"];
pub const NISAR_PROFILE_ID: &str = "asf.nisar.l.rslc.provisional.v1";
pub const NISAR_LOGICAL_DATASET: &str = "sar.nisar.l.rslc.provisional.v1";
pub const NISAR_PROVIDER_ID: &str = "asf.nasa.earthdata";
pub const NISAR_DATASET_SELECTOR: &str = "NISAR";
pub const NISAR_NATIVE_COLLECTION_ID: &str = "C2853145197-ASF";
pub const NISAR_COLLECTION_SHORT_NAME: &str = "NISAR_L1_RSLC_PROVISIONAL_V1";
pub const NISAR_COLLECTION_VERSION: &str = "1";
pub const NISAR_IDENTITY_RULE_ID: &str = "asf.nisar.cmr-granule.v1";
pub const NISAR_FILENAME_RULE_ID: &str = "asf.nisar.rslc-hdf5.v1";
pub const NISAR_RESOLVER_POLICY_ID_V1: &str = "asf.nisar.earthdata-complete-hdf5.v1";
pub const NISAR_RESOLVER_POLICY_ID_V2: &str = "asf.nisar.earthdata-complete-hdf5.v2";
pub const NISAR_RESOLVER_POLICY_ID: &str = NISAR_RESOLVER_POLICY_ID_V2;
pub const NISAR_P05023_CRID: &str = "P05023";
pub const NISAR_P05023_PRODUCT_SPECIFICATION_VERSION: &str = "1.5.0";
pub const NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION: u32 = 1;
pub const NISAR_REQUIRED_PRODUCT_TYPE: &str = "RSLC";
pub const NISAR_SCIENCE_LEVEL: &str = "L1";
pub const NISAR_QUALITY_STATUS: &str = "Provisional";
pub const NISAR_PRODUCTION_CONFIGURATION: &str = "PR";
pub const NISAR_BAND: &str = "L";
pub const NISAR_FRAME_COVERAGES: &[&str] = &["FULL", "PARTIAL"];
pub const NISAR_FREQUENCY_A_POLARIZATIONS: &[&str] = &["HH", "HH+HV", "VV+VH", "HH+HV+VH+VV"];
pub const NISAR_FREQUENCY_B_POLARIZATIONS: &[&str] = &["HH", "HH+HV", "VV", "VV+VH"];
pub const NISAR_RANGE_BANDWIDTHS: &[&str] = &["5", "20+5", "40+5", "77"];
pub const NISAR_FLIGHT_DIRECTIONS: &[&str] = &["ASCENDING", "DESCENDING"];
pub const NISAR_LOOK_DIRECTIONS: &[&str] = &["LEFT", "RIGHT"];

pub const BHOONIDHI_PROFILE_ID: &str = crate::bhoonidhi::BHOONIDHI_PROFILE_ID;
pub const BHOONIDHI_LOGICAL_DATASET: &str = crate::bhoonidhi::BHOONIDHI_LOGICAL_DATASET;
pub const BHOONIDHI_PROVIDER_ID: &str = crate::bhoonidhi::BHOONIDHI_PROVIDER_ID;
pub const BHOONIDHI_NATIVE_COLLECTION_ID: &str = crate::bhoonidhi::BHOONIDHI_COLLECTION_ID;
pub const BHOONIDHI_REQUIRED_PRODUCT_TYPE: &str = "RSLC";
pub const BHOONIDHI_IDENTITY_RULE_ID: &str = crate::bhoonidhi::BHOONIDHI_IDENTITY_RULE_ID;
pub const BHOONIDHI_FILENAME_RULE_ID: &str = crate::bhoonidhi::BHOONIDHI_FILENAME_RULE_ID;
pub const BHOONIDHI_RESOLVER_POLICY_ID: &str = crate::bhoonidhi::BHOONIDHI_RESOLVER_POLICY_ID;

pub const OPERA_S1_PROFILE_ID: &str = "asf.opera.s1.collection.v1";
pub const OPERA_S1_LOGICAL_DATASET: &str = "sar.opera.s1.disp.v1";
pub const OPERA_S1_PROVIDER_ID: &str = "asf.nasa.earthdata";
pub const OPERA_S1_NATIVE_COLLECTION_ID: &str = "C2834279090-ASF";
pub const OPERA_S1_REQUIRED_PRODUCT_TYPE: &str = "DISP-S1";
pub const OPERA_S1_IDENTITY_RULE_ID: &str = "asf.opera.cmr-granule.v1";
pub const OPERA_S1_FILENAME_RULE_ID: &str = "asf.opera.netcdf-filename.v1";
pub const OPERA_S1_RESOLVER_POLICY_ID: &str = "asf.opera.earthdata-complete-nc.v1";

pub const LICSAR_PROFILE_ID: &str = "comet.licsar.collection.v1";
pub const LICSAR_LOGICAL_DATASET: &str = "sar.comet.licsar.interferogram.v1";
pub const LICSAR_PROVIDER_ID: &str = "comet.licsar.portal";
pub const LICSAR_NATIVE_COLLECTION_ID: &str = "LiCSAR_products.public";
pub const LICSAR_REQUIRED_PRODUCT_TYPE: &str = "GEOTIFF_UNW";
pub const LICSAR_IDENTITY_RULE_ID: &str = "comet.licsar.identity.v1";
pub const LICSAR_FILENAME_RULE_ID: &str = "comet.licsar.geotiff-filename.v1";
pub const LICSAR_RESOLVER_POLICY_ID: &str = "comet.licsar.direct-http-geotiff.v1";

pub const ASTER_GDEM_V3_PROFILE_ID: &str = "lpdaac.aster-gdem.v3.collection.v1";
pub const ASTER_GDEM_V3_LOGICAL_DATASET: &str = "dem.aster-gdem.v3";
pub const ASTER_GDEM_V3_PROVIDER_ID: &str = "asf.nasa.earthdata";
pub const ASTER_GDEM_V3_NATIVE_COLLECTION_ID: &str = "ASTGTM.v003";
pub const ASTER_GDEM_V3_REQUIRED_PRODUCT_TYPE: &str = "ASTER_GDEM_V3";
pub const ASTER_GDEM_V3_IDENTITY_RULE_ID: &str = "lpdaac.aster-gdem.v3.identity.v1";
pub const ASTER_GDEM_V3_FILENAME_RULE_ID: &str = "lpdaac.aster-gdem.v3.filename.v1";
pub const ASTER_GDEM_V3_RESOLVER_POLICY_ID: &str = "lpdaac.aster-gdem.v3.earthdata-cloud-cog.v1";

pub const USGS_EARTHQUAKE_PROFILE_ID: &str = "usgs.earthquake.collection.v1";
pub const USGS_EARTHQUAKE_LOGICAL_DATASET: &str = "event.usgs.earthquake.v1";
pub const USGS_EARTHQUAKE_PROVIDER_ID: &str = "usgs.earthquake.fdsn";
pub const USGS_EARTHQUAKE_NATIVE_COLLECTION_ID: &str = "earthquake";
pub const USGS_EARTHQUAKE_REQUIRED_PRODUCT_TYPE: &str = "GeoJSON";
pub const USGS_EARTHQUAKE_IDENTITY_RULE_ID: &str = "usgs.earthquake.event-id.v1";
pub const USGS_EARTHQUAKE_FILENAME_RULE_ID: &str = "usgs.earthquake.geojson.v1";
pub const USGS_EARTHQUAKE_RESOLVER_POLICY_ID: &str = "usgs.earthquake.fdsn-api.v1";

pub const NGL_GNSS_PROFILE_ID: &str = "unr.ngl.gnss.collection.v1";
pub const NGL_GNSS_LOGICAL_DATASET: &str = "geodesy.ngl.gnss.v1";
pub const NGL_GNSS_PROVIDER_ID: &str = "unr.ngl.geodesy";
pub const NGL_GNSS_NATIVE_COLLECTION_ID: &str = "tenv3";
pub const NGL_GNSS_REQUIRED_PRODUCT_TYPE: &str = "tenv3";
pub const NGL_GNSS_IDENTITY_RULE_ID: &str = "unr.ngl.station-tenv3.v1";
pub const NGL_GNSS_FILENAME_RULE_ID: &str = "unr.ngl.tenv3-filename.v1";
pub const NGL_GNSS_RESOLVER_POLICY_ID: &str = "unr.ngl.http-tenv3.v1";

pub const ERA5_PROFILE_ID: &str = "ecmwf.era5.arco.collection.v1";
pub const ERA5_LOGICAL_DATASET: &str = "atmospheric.era5.reanalysis.v1";
pub const ERA5_PROVIDER_ID: &str = "ecmwf.era5.arco";
pub const ERA5_NATIVE_COLLECTION_ID: &str = "ARCO_ERA5_HOURLY";
pub const ERA5_REQUIRED_PRODUCT_TYPE: &str = "ERA5_HOURLY";
pub const ERA5_IDENTITY_RULE_ID: &str = "ecmwf.era5.hourly-id.v1";
pub const ERA5_FILENAME_RULE_ID: &str = "ecmwf.era5.netcdf-filename.v1";
pub const ERA5_RESOLVER_POLICY_ID: &str = "ecmwf.era5.arco-zarr.v1";

pub const IGS_GIM_PROFILE_ID: &str = "cddis.ionex.gim.collection.v1";
pub const IGS_GIM_LOGICAL_DATASET: &str = "ionospheric.igs.gim.v1";
pub const IGS_GIM_PROVIDER_ID: &str = "asf.nasa.earthdata";
pub const IGS_GIM_NATIVE_COLLECTION_ID: &str = "CDDIS_IONEX_GIM";
pub const IGS_GIM_REQUIRED_PRODUCT_TYPE: &str = "IONEX";
pub const IGS_GIM_IDENTITY_RULE_ID: &str = "cddis.ionex.gim-id.v1";
pub const IGS_GIM_FILENAME_RULE_ID: &str = "cddis.ionex.filename.v1";
pub const IGS_GIM_RESOLVER_POLICY_ID: &str = "cddis.ionex.https.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NisarRslcProductSpecificationProfile {
    pub registry_version: u32,
    pub crid: &'static str,
    pub product_specification_version: &'static str,
    pub resolver_policy_id: &'static str,
}

static NISAR_P05023_PRODUCT_SPECIFICATION_PROFILE: NisarRslcProductSpecificationProfile =
    NisarRslcProductSpecificationProfile {
        registry_version: NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION,
        crid: NISAR_P05023_CRID,
        product_specification_version: NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
        resolver_policy_id: NISAR_RESOLVER_POLICY_ID_V2,
    };

/// Closed bridge for CMR records that expose a CRID but do not expose the
/// productSpecificationVersion stored in the complete HDF5 product.
#[derive(Clone, Copy, Debug, Default)]
pub struct NisarRslcProductSpecificationProfileRegistry;

impl NisarRslcProductSpecificationProfileRegistry {
    pub const fn version(&self) -> u32 {
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    }

    pub fn for_crid(&self, crid: &str) -> Option<&'static NisarRslcProductSpecificationProfile> {
        match crid {
            NISAR_P05023_CRID => Some(&NISAR_P05023_PRODUCT_SPECIFICATION_PROFILE),
            _ => None,
        }
    }
}

const ALOS2_WBS_POLARIZATIONS: &[&str] = &["HH"];
const ALOS2_WBD_POLARIZATIONS: &[&str] = &["HH", "HV"];

pub fn alos2_expected_polarizations(beam_mode: &str) -> Option<&'static [&'static str]> {
    match beam_mode {
        "WBS" => Some(ALOS2_WBS_POLARIZATIONS),
        "WBD" => Some(ALOS2_WBD_POLARIZATIONS),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRoleMapping {
    pub native_role: &'static str,
    pub normalized_role: AssetRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionProfile {
    pub profile_id: &'static str,
    pub logical_dataset: &'static str,
    pub provider_id: &'static str,
    pub native_collection_id: &'static str,
    pub supplemental_provider_id: &'static str,
    pub supplemental_collection_id: &'static str,
    pub required_product_type: &'static str,
    pub identity_rule_id: &'static str,
    pub filename_rule_id: &'static str,
    pub resolver_policy_id: &'static str,
    pub delivery_model: CatalogDeliveryModel,
    pub production_enabled: bool,
    native_role_mappings: &'static [NativeRoleMapping],
}

impl CollectionProfile {
    pub fn normalized_role_for_native_roles(&self, native_roles: &[String]) -> Option<AssetRole> {
        let [native_role] = native_roles else {
            return None;
        };
        self.native_role_mappings
            .iter()
            .find(|mapping| mapping.native_role == native_role)
            .map(|mapping| mapping.normalized_role.clone())
    }
}

/// Explicitly describes the future USGS M2M session boundary without creating
/// a credential provider or accepting any credential material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatSessionBoundary {
    UsgsM2m,
}

impl LandsatSessionBoundary {
    pub const fn profile_id(self) -> &'static str {
        match self {
            Self::UsgsM2m => USGS_M2M_SESSION_PROFILE_ID,
        }
    }

    pub fn matches_profile(self, profile_id: &str) -> bool {
        profile_id == self.profile_id()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatSearchFilter {
    Aoi,
    AcquisitionDateRange,
    CloudCoverLte,
    Spacecraft,
    ProcessingLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatSpacecraft {
    Landsat8,
    Landsat9,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatProcessingLevel {
    L2Sp,
    L2Sr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatDeliveryOption {
    FullBundle,
    SelectedBands,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LandsatPreviewStrategy {
    ExactSceneBrowse,
}

/// The original non-executable, no-credential Landsat reservation. It does not
/// duplicate the live-accepted identity owned by `usgs_m2m::router` and grants
/// no scene-search, preview, asset, or transfer authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LandsatC2L2Foundation {
    pub profile_id: &'static str,
    pub logical_dataset: &'static str,
    pub provider_id: &'static str,
    pub upstream_dataset_id: Option<&'static str>,
    pub session_boundary: LandsatSessionBoundary,
    pub supported_filters: &'static [LandsatSearchFilter],
    pub spacecraft: &'static [LandsatSpacecraft],
    pub processing_levels: &'static [LandsatProcessingLevel],
    pub delivery_options: &'static [LandsatDeliveryOption],
    pub preview_strategy: LandsatPreviewStrategy,
}

impl LandsatC2L2Foundation {
    pub const fn is_executable(&self) -> bool {
        false
    }

    pub const fn permits_runtime_credential_binding(&self) -> bool {
        false
    }
}

const SENTINEL2_SAFE_ROLE_MAPPINGS: &[NativeRoleMapping] = &[
    NativeRoleMapping {
        native_role: "archive",
        normalized_role: AssetRole::Primary,
    },
    NativeRoleMapping {
        native_role: "thumbnail",
        normalized_role: AssetRole::Thumbnail,
    },
    NativeRoleMapping {
        native_role: "overview",
        normalized_role: AssetRole::Browse,
    },
];

const ALOS2_ROLE_MAPPINGS: &[NativeRoleMapping] = &[NativeRoleMapping {
    native_role: "archive",
    normalized_role: AssetRole::Primary,
}];

static SENTINEL2_L2A_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL2_L2A_PROFILE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-2",
    supplemental_provider_id: "copernicus.dataspace.stac",
    supplemental_collection_id: "sentinel-2-l2a",
    required_product_type: "S2MSI2A",
    identity_rule_id: CDSE_SENTINEL2_L2A_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL2_L2A_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL2_L2A_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

static MPC_SENTINEL2_L2A_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_SENTINEL2_L2A_PROFILE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: MPC_SENTINEL2_L2A_PROVIDER_ID,
    native_collection_id: MPC_SENTINEL2_L2A_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "S2MSI2A",
    identity_rule_id: MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
    filename_rule_id: MPC_SENTINEL2_L2A_FILENAME_RULE_ID,
    resolver_policy_id: MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
    // Search-only by design. No execution-policy registry entry or delivery
    // capability is attached to this profile.
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static MPC_SENTINEL1_GRD_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_SENTINEL1_GRD_PROFILE_ID,
    logical_dataset: MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
    native_collection_id: MPC_SENTINEL1_GRD_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "SENTINEL1_GRD_COG",
    identity_rule_id: MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
    filename_rule_id: MPC_SENTINEL1_GRD_FILENAME_RULE_ID,
    resolver_policy_id: MPC_SENTINEL1_GRD_ASSET_POLICY_ID,
    // Search freezes descriptive COG identities only. Per-asset execution
    // authority is minted later from exact re-resolved evidence.
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static MPC_COPDEM_GLO30_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_COPDEM_GLO30_PROFILE_ID,
    logical_dataset: MPC_COPDEM_GLO30_LOGICAL_DATASET,
    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
    native_collection_id: MPC_COPDEM_GLO30_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "COPDEM_GLO30_COG",
    identity_rule_id: MPC_REVIEWED_STAC_IDENTITY_RULE_ID,
    filename_rule_id: MPC_COPDEM_GLO30_FILENAME_RULE_ID,
    resolver_policy_id: MPC_COPDEM_GLO30_ASSET_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

/// Issue #3 R2 B5: HLS v2.0 is search-only; the resolver policy names the
/// absence of any transfer authority.
static MPC_HLS_L30_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_HLS_L30_PROFILE_ID,
    logical_dataset: MPC_HLS_L30_LOGICAL_DATASET,
    provider_id: MPC_HLS_PROVIDER_ID,
    native_collection_id: MPC_HLS_L30_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "HLSL30",
    identity_rule_id: MPC_HLS_IDENTITY_RULE_ID,
    filename_rule_id: MPC_HLS_FILENAME_RULE_ID,
    resolver_policy_id: MPC_HLS_SEARCH_ONLY_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static MPC_HLS_S30_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_HLS_S30_PROFILE_ID,
    logical_dataset: MPC_HLS_S30_LOGICAL_DATASET,
    provider_id: MPC_HLS_PROVIDER_ID,
    native_collection_id: MPC_HLS_S30_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "HLSS30",
    identity_rule_id: MPC_HLS_IDENTITY_RULE_ID,
    filename_rule_id: MPC_HLS_FILENAME_RULE_ID,
    resolver_policy_id: MPC_HLS_SEARCH_ONLY_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static SENTINEL2_L1C_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL2_L1C_PROFILE_ID,
    logical_dataset: "optical.sentinel-2.l1c",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-2",
    supplemental_provider_id: "copernicus.dataspace.stac",
    supplemental_collection_id: "sentinel-2-l1c",
    required_product_type: "S2MSI1C",
    identity_rule_id: CDSE_SENTINEL2_L1C_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL2_L1C_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL2_L1C_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

pub static SENTINEL3_SLSTR_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL3_SLSTR_PROFILE_ID,
    logical_dataset: "optical.sentinel-3.slstr",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-3",
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "SL_2_LST___",
    identity_rule_id: CDSE_SENTINEL3_SLSTR_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL3_SLSTR_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL3_SLSTR_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

pub static SENTINEL3_OLCI_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL3_OLCI_PROFILE_ID,
    logical_dataset: "optical.sentinel-3.olci",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-3",
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "OL_2_LFR___",
    identity_rule_id: CDSE_SENTINEL3_OLCI_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL3_OLCI_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL3_OLCI_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

pub static SENTINEL3_SRAL_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL3_SRAL_PROFILE_ID,
    logical_dataset: "sar.sentinel-3.sral",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-3",
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "SR_2_LAN___",
    identity_rule_id: CDSE_SENTINEL3_SRAL_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL3_SRAL_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL3_SRAL_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

pub static SENTINEL3_STANDARD_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: SENTINEL3_STANDARD_PROFILE_ID,
    logical_dataset: "optical.sentinel-3.standard",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-3",
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "SENTINEL3_STANDARD",
    identity_rule_id: CDSE_SENTINEL3_STANDARD_IDENTITY_RULE_ID,
    filename_rule_id: CDSE_SENTINEL3_STANDARD_ARCHIVE_FILENAME_RULE_ID,
    resolver_policy_id: CDSE_SENTINEL3_STANDARD_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: SENTINEL2_SAFE_ROLE_MAPPINGS,
};

const LANDSAT_C2_L2_ROLE_MAPPINGS: &[NativeRoleMapping] = &[NativeRoleMapping {
    native_role: "full_bundle",
    normalized_role: AssetRole::Primary,
}];

static LANDSAT_C2_L2_SCENE_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: LANDSAT_C2_L2_PROVIDER_ID,
    native_collection_id: LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "LANDSAT_C2_L2_SCENE",
    identity_rule_id: USGS_LANDSAT_C2_L2_IDENTITY_RULE_ID,
    filename_rule_id: USGS_LANDSAT_C2_L2_BUNDLE_FILENAME_RULE_ID,
    resolver_policy_id: USGS_LANDSAT_C2_L2_BUNDLE_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: landsat_scene_search_live_accepted(),
    native_role_mappings: LANDSAT_C2_L2_ROLE_MAPPINGS,
};

static MPC_LANDSAT_SCENE_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: MPC_LANDSAT_SCENE_PROFILE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: MPC_LANDSAT_PROVIDER_ID,
    native_collection_id: MPC_LANDSAT_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "LANDSAT_C2_L2_SCENE",
    identity_rule_id: MPC_LANDSAT_IDENTITY_RULE_ID,
    filename_rule_id: MPC_LANDSAT_FILENAME_RULE_ID,
    resolver_policy_id: MPC_LANDSAT_ASSET_POLICY_ID,
    // Keep the existing scene-first contract. Executable per-file delivery is
    // separately admitted by its exact asset-set policy, never by this label.
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static EARTH_SEARCH_LANDSAT_SCENE_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: EARTH_SEARCH_LANDSAT_PROVIDER_ID,
    native_collection_id: EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "LANDSAT_C2_L2_SCENE",
    identity_rule_id: EARTH_SEARCH_LANDSAT_IDENTITY_RULE_ID,
    filename_rule_id: EARTH_SEARCH_LANDSAT_FILENAME_RULE_ID,
    // Search-only by design. No execution-policy registry entry exists for
    // this id and no AssetSet capability references the Earth Search route.
    resolver_policy_id: EARTH_SEARCH_LANDSAT_SEARCH_ONLY_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static ALOS2_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: ALOS2_PROFILE_ID,
    logical_dataset: ALOS2_LOGICAL_DATASET,
    provider_id: ALOS2_PROVIDER_ID,
    native_collection_id: ALOS2_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: ALOS2_COLLECTION_SHORT_NAME,
    identity_rule_id: ALOS2_IDENTITY_RULE_ID,
    filename_rule_id: ALOS2_FILENAME_RULE_ID,
    resolver_policy_id: ALOS2_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: ALOS2_ROLE_MAPPINGS,
};

static NISAR_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: NISAR_PROFILE_ID,
    logical_dataset: NISAR_LOGICAL_DATASET,
    provider_id: NISAR_PROVIDER_ID,
    native_collection_id: NISAR_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: NISAR_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: NISAR_IDENTITY_RULE_ID,
    filename_rule_id: NISAR_FILENAME_RULE_ID,
    resolver_policy_id: NISAR_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

const BHOONIDHI_SAFE_ROLE_MAPPINGS: &[NativeRoleMapping] = &[
    NativeRoleMapping {
        native_role: "metadata",
        normalized_role: AssetRole::Metadata,
    },
    NativeRoleMapping {
        native_role: "thumbnail",
        normalized_role: AssetRole::Thumbnail,
    },
];

static BHOONIDHI_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: BHOONIDHI_PROFILE_ID,
    logical_dataset: BHOONIDHI_LOGICAL_DATASET,
    provider_id: BHOONIDHI_PROVIDER_ID,
    native_collection_id: BHOONIDHI_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: BHOONIDHI_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: BHOONIDHI_IDENTITY_RULE_ID,
    filename_rule_id: BHOONIDHI_FILENAME_RULE_ID,
    resolver_policy_id: BHOONIDHI_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: BHOONIDHI_SAFE_ROLE_MAPPINGS,
};

static OPERA_S1_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: OPERA_S1_PROFILE_ID,
    logical_dataset: OPERA_S1_LOGICAL_DATASET,
    provider_id: OPERA_S1_PROVIDER_ID,
    native_collection_id: OPERA_S1_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: OPERA_S1_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: OPERA_S1_IDENTITY_RULE_ID,
    filename_rule_id: OPERA_S1_FILENAME_RULE_ID,
    resolver_policy_id: OPERA_S1_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static LICSAR_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: LICSAR_PROFILE_ID,
    logical_dataset: LICSAR_LOGICAL_DATASET,
    provider_id: LICSAR_PROVIDER_ID,
    native_collection_id: LICSAR_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: LICSAR_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: LICSAR_IDENTITY_RULE_ID,
    filename_rule_id: LICSAR_FILENAME_RULE_ID,
    resolver_policy_id: LICSAR_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static ASTER_GDEM_V3_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: ASTER_GDEM_V3_PROFILE_ID,
    logical_dataset: ASTER_GDEM_V3_LOGICAL_DATASET,
    provider_id: ASTER_GDEM_V3_PROVIDER_ID,
    native_collection_id: ASTER_GDEM_V3_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: ASTER_GDEM_V3_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: ASTER_GDEM_V3_IDENTITY_RULE_ID,
    filename_rule_id: ASTER_GDEM_V3_FILENAME_RULE_ID,
    resolver_policy_id: ASTER_GDEM_V3_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static USGS_EARTHQUAKE_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: USGS_EARTHQUAKE_PROFILE_ID,
    logical_dataset: USGS_EARTHQUAKE_LOGICAL_DATASET,
    provider_id: USGS_EARTHQUAKE_PROVIDER_ID,
    native_collection_id: USGS_EARTHQUAKE_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: USGS_EARTHQUAKE_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: USGS_EARTHQUAKE_IDENTITY_RULE_ID,
    filename_rule_id: USGS_EARTHQUAKE_FILENAME_RULE_ID,
    resolver_policy_id: USGS_EARTHQUAKE_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static NGL_GNSS_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: NGL_GNSS_PROFILE_ID,
    logical_dataset: NGL_GNSS_LOGICAL_DATASET,
    provider_id: NGL_GNSS_PROVIDER_ID,
    native_collection_id: NGL_GNSS_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: NGL_GNSS_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: NGL_GNSS_IDENTITY_RULE_ID,
    filename_rule_id: NGL_GNSS_FILENAME_RULE_ID,
    resolver_policy_id: NGL_GNSS_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static ERA5_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: ERA5_PROFILE_ID,
    logical_dataset: ERA5_LOGICAL_DATASET,
    provider_id: ERA5_PROVIDER_ID,
    native_collection_id: ERA5_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: ERA5_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: ERA5_IDENTITY_RULE_ID,
    filename_rule_id: ERA5_FILENAME_RULE_ID,
    resolver_policy_id: ERA5_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static IGS_GIM_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: IGS_GIM_PROFILE_ID,
    logical_dataset: IGS_GIM_LOGICAL_DATASET,
    provider_id: IGS_GIM_PROVIDER_ID,
    native_collection_id: IGS_GIM_NATIVE_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: IGS_GIM_REQUIRED_PRODUCT_TYPE,
    identity_rule_id: IGS_GIM_IDENTITY_RULE_ID,
    filename_rule_id: IGS_GIM_FILENAME_RULE_ID,
    resolver_policy_id: IGS_GIM_RESOLVER_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[],
};

static EARTH_SEARCH_SENTINEL2_L2A_PROFILE: CollectionProfile = CollectionProfile {
    profile_id: EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    native_collection_id: EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
    supplemental_provider_id: "none",
    supplemental_collection_id: "none",
    required_product_type: "S2MSI2A",
    identity_rule_id: EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    filename_rule_id: EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID,
    resolver_policy_id: EARTH_SEARCH_SENTINEL2_L2A_BAND_ASSET_POLICY_ID,
    delivery_model: CatalogDeliveryModel::ArchiveFirst,
    production_enabled: true,
    native_role_mappings: &[NativeRoleMapping {
        native_role: "thumbnail",
        normalized_role: AssetRole::Thumbnail,
    }],
};

/// Exact collection profiles form one source-keyed registry. Adding another
/// channel for an existing product extends this table instead of teaching
/// `for_source` another product-specific branch.
static COLLECTION_PROFILES: [&CollectionProfile; 25] = [
    &SENTINEL2_L2A_PROFILE,
    &MPC_SENTINEL2_L2A_PROFILE,
    &EARTH_SEARCH_SENTINEL2_L2A_PROFILE,
    &MPC_SENTINEL1_GRD_PROFILE,
    &MPC_COPDEM_GLO30_PROFILE,
    &MPC_HLS_L30_PROFILE,
    &MPC_HLS_S30_PROFILE,
    &SENTINEL2_L1C_PROFILE,
    &LANDSAT_C2_L2_SCENE_PROFILE,
    &MPC_LANDSAT_SCENE_PROFILE,
    &EARTH_SEARCH_LANDSAT_SCENE_PROFILE,
    &ALOS2_PROFILE,
    &NISAR_PROFILE,
    &BHOONIDHI_PROFILE,
    &OPERA_S1_PROFILE,
    &LICSAR_PROFILE,
    &ASTER_GDEM_V3_PROFILE,
    &USGS_EARTHQUAKE_PROFILE,
    &NGL_GNSS_PROFILE,
    &ERA5_PROFILE,
    &IGS_GIM_PROFILE,
    &SENTINEL3_SLSTR_PROFILE,
    &SENTINEL3_OLCI_PROFILE,
    &SENTINEL3_SRAL_PROFILE,
    &SENTINEL3_STANDARD_PROFILE,
];

const LANDSAT_C2_L2_SUPPORTED_FILTERS: &[LandsatSearchFilter] = &[
    LandsatSearchFilter::Aoi,
    LandsatSearchFilter::AcquisitionDateRange,
    LandsatSearchFilter::CloudCoverLte,
    LandsatSearchFilter::Spacecraft,
    LandsatSearchFilter::ProcessingLevel,
];

const LANDSAT_C2_L2_SPACECRAFT: &[LandsatSpacecraft] =
    &[LandsatSpacecraft::Landsat8, LandsatSpacecraft::Landsat9];

const LANDSAT_C2_L2_PROCESSING_LEVELS: &[LandsatProcessingLevel] =
    &[LandsatProcessingLevel::L2Sp, LandsatProcessingLevel::L2Sr];

const LANDSAT_C2_L2_DELIVERY_OPTIONS: &[LandsatDeliveryOption] =
    &[LandsatDeliveryOption::FullBundle];

static LANDSAT_C2_L2_FOUNDATION: LandsatC2L2Foundation = LandsatC2L2Foundation {
    profile_id: LANDSAT_C2_L2_FOUNDATION_PROFILE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: LANDSAT_C2_L2_PROVIDER_ID,
    upstream_dataset_id: None,
    session_boundary: LandsatSessionBoundary::UsgsM2m,
    supported_filters: LANDSAT_C2_L2_SUPPORTED_FILTERS,
    spacecraft: LANDSAT_C2_L2_SPACECRAFT,
    processing_levels: LANDSAT_C2_L2_PROCESSING_LEVELS,
    delivery_options: LANDSAT_C2_L2_DELIVERY_OPTIONS,
    preview_strategy: LandsatPreviewStrategy::ExactSceneBrowse,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredFixtureReservation {
    pub logical_dataset: &'static str,
    pub fixture_only: bool,
}

const DEFERRED_FIXTURE_RESERVATIONS: &[DeferredFixtureReservation] = &[
    DeferredFixtureReservation {
        logical_dataset: "optical.sentinel-3.olci",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.sentinel-3.slstr",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.hls-s30",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.hls-l30",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.modis",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.viirs",
        fixture_only: true,
    },
    DeferredFixtureReservation {
        logical_dataset: "optical.aster",
        fixture_only: true,
    },
];

#[derive(Clone, Copy, Debug, Default)]
pub struct CollectionProfileRegistry;

impl CollectionProfileRegistry {
    pub fn get(&self, profile_id: &str) -> Option<&'static CollectionProfile> {
        let mut matches = COLLECTION_PROFILES
            .iter()
            .copied()
            .filter(|profile| profile.profile_id == profile_id);
        let profile = matches.next()?;
        matches.next().is_none().then_some(profile)
    }

    /// Exact source lookup for new requests and persisted identities. A product
    /// label alone cannot authorize another provider's collection or policy.
    pub fn for_source(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Option<&'static CollectionProfile> {
        let mut matches = COLLECTION_PROFILES.iter().copied().filter(|profile| {
            profile.logical_dataset == logical_dataset
                && profile.provider_id == provider_id
                && profile.native_collection_id == native_collection_id
        });
        let profile = matches.next()?;
        // Duplicate exact bindings are an authority ambiguity. Fail closed
        // instead of selecting one by declaration order.
        matches.next().is_none().then_some(profile)
    }

    /// Legacy single-source projection. New multi-source consumers must use
    /// `for_source`; retaining this projection does not rewrite old identities.
    pub fn for_logical_dataset(&self, logical_dataset: &str) -> Option<&'static CollectionProfile> {
        match logical_dataset {
            value if value == SENTINEL2_L2A_PROFILE.logical_dataset => Some(&SENTINEL2_L2A_PROFILE),
            value if value == SENTINEL2_L1C_PROFILE.logical_dataset => Some(&SENTINEL2_L1C_PROFILE),
            value if value == LANDSAT_C2_L2_SCENE_PROFILE.logical_dataset => {
                Some(&LANDSAT_C2_L2_SCENE_PROFILE)
            }
            value if value == MPC_SENTINEL1_GRD_PROFILE.logical_dataset => {
                Some(&MPC_SENTINEL1_GRD_PROFILE)
            }
            value if value == MPC_COPDEM_GLO30_PROFILE.logical_dataset => {
                Some(&MPC_COPDEM_GLO30_PROFILE)
            }
            value if value == ALOS2_PROFILE.logical_dataset => Some(&ALOS2_PROFILE),
            value if value == NISAR_PROFILE.logical_dataset => Some(&NISAR_PROFILE),
            value if value == BHOONIDHI_PROFILE.logical_dataset => Some(&BHOONIDHI_PROFILE),
            value if value == OPERA_S1_PROFILE.logical_dataset => Some(&OPERA_S1_PROFILE),
            value if value == LICSAR_PROFILE.logical_dataset => Some(&LICSAR_PROFILE),
            value if value == ASTER_GDEM_V3_PROFILE.logical_dataset => Some(&ASTER_GDEM_V3_PROFILE),
            value if value == USGS_EARTHQUAKE_PROFILE.logical_dataset => {
                Some(&USGS_EARTHQUAKE_PROFILE)
            }
            value if value == NGL_GNSS_PROFILE.logical_dataset => Some(&NGL_GNSS_PROFILE),
            value if value == ERA5_PROFILE.logical_dataset => Some(&ERA5_PROFILE),
            value if value == IGS_GIM_PROFILE.logical_dataset => Some(&IGS_GIM_PROFILE),
            value if value == SENTINEL3_STANDARD_PROFILE.logical_dataset => {
                Some(&SENTINEL3_STANDARD_PROFILE)
            }
            _ => None,
        }
    }

    pub fn deferred_fixture_reservations(&self) -> &'static [DeferredFixtureReservation] {
        DEFERRED_FIXTURE_RESERVATIONS
    }

    pub fn landsat_c2_l2_foundation(&self) -> &'static LandsatC2L2Foundation {
        &LANDSAT_C2_L2_FOUNDATION
    }
}
