use super::{
    ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROFILE_ID,
    BHOONIDHI_PROVIDER_ID, EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID, EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET, LANDSAT_C2_L2_NATIVE_COLLECTION_ID, LANDSAT_C2_L2_PROVIDER_ID,
    LANDSAT_C2_L2_SCENE_PROFILE_ID, MPC_SENTINEL2_L2A_COLLECTION_ID, MPC_SENTINEL2_L2A_PROFILE_ID,
    MPC_SENTINEL2_L2A_PROVIDER_ID, NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID,
    NISAR_PROFILE_ID, NISAR_PROVIDER_ID, SENTINEL2_L1C_PROFILE_ID, SENTINEL2_L2A_PROFILE_ID,
};
use super::{
    EARTH_SEARCH_LANDSAT_COLLECTION_ID, EARTH_SEARCH_LANDSAT_PROVIDER_ID,
    EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID, MPC_LANDSAT_COLLECTION_ID, MPC_LANDSAT_PROVIDER_ID,
    MPC_LANDSAT_SCENE_PROFILE_ID,
};
use crate::planetary_computer_asset::{
    MPC_COPDEM_GLO30_COLLECTION_ID, MPC_COPDEM_GLO30_LOGICAL_DATASET, MPC_COPDEM_GLO30_PROFILE_ID,
    MPC_SENTINEL1_GRD_COLLECTION_ID, MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    MPC_SENTINEL1_GRD_PROFILE_ID, PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
};
use crate::usgs_m2m::landsat_scene_search_live_accepted;

use super::mpc_hls::{
    MPC_HLS_L30_COLLECTION_ID, MPC_HLS_L30_LOGICAL_DATASET, MPC_HLS_L30_PROFILE_ID,
    MPC_HLS_L30_ROUTE_ID, MPC_HLS_PROVIDER_ID, MPC_HLS_S30_COLLECTION_ID,
    MPC_HLS_S30_LOGICAL_DATASET, MPC_HLS_S30_PROFILE_ID, MPC_HLS_S30_ROUTE_ID,
};

use super::collection_profile::{
    ASTER_GDEM_V3_LOGICAL_DATASET, ASTER_GDEM_V3_NATIVE_COLLECTION_ID, ASTER_GDEM_V3_PROFILE_ID,
    ASTER_GDEM_V3_PROVIDER_ID, ERA5_LOGICAL_DATASET, ERA5_NATIVE_COLLECTION_ID, ERA5_PROFILE_ID,
    ERA5_PROVIDER_ID, IGS_GIM_LOGICAL_DATASET, IGS_GIM_NATIVE_COLLECTION_ID, IGS_GIM_PROFILE_ID,
    IGS_GIM_PROVIDER_ID, LICSAR_LOGICAL_DATASET, LICSAR_NATIVE_COLLECTION_ID, LICSAR_PROFILE_ID,
    LICSAR_PROVIDER_ID, NGL_GNSS_LOGICAL_DATASET, NGL_GNSS_NATIVE_COLLECTION_ID,
    NGL_GNSS_PROFILE_ID, NGL_GNSS_PROVIDER_ID, OPERA_S1_LOGICAL_DATASET,
    OPERA_S1_NATIVE_COLLECTION_ID, OPERA_S1_PROFILE_ID, OPERA_S1_PROVIDER_ID,
    SENTINEL3_STANDARD_PROFILE_ID, USGS_EARTHQUAKE_LOGICAL_DATASET,
    USGS_EARTHQUAKE_NATIVE_COLLECTION_ID, USGS_EARTHQUAKE_PROFILE_ID, USGS_EARTHQUAKE_PROVIDER_ID,
};

pub const SENTINEL2_L2A_ROUTE_ID: &str = "cdse.sentinel-2.l2a.route.v1";
pub const MPC_SENTINEL2_L2A_ROUTE_ID: &str = "planetary-computer.sentinel-2-l2a.route.v1";
pub const EARTH_SEARCH_SENTINEL2_L2A_ROUTE_ID: &str =
    "element84.earth-search.sentinel-2-l2a.route.v1";
pub const MPC_SENTINEL1_GRD_ROUTE_ID: &str = "planetary-computer.sentinel-1-grd.cog.route.v1";
pub const MPC_COPDEM_GLO30_ROUTE_ID: &str = "planetary-computer.cop-dem-glo-30.cog.route.v1";
pub const SENTINEL2_L1C_ROUTE_ID: &str = "cdse.sentinel-2.l1c.route.v1";
pub const SENTINEL3_STANDARD_ROUTE_ID: &str = "route.cdse.sentinel-3.standard.v1";
pub const LANDSAT_C2_L2_SCENE_ROUTE_ID: &str = "usgs.eros.m2m.landsat.c2.l2.scene.route.v1";
pub const LANDSAT_TM_C2_L2_SCENE_ROUTE_ID: &str = "usgs.eros.m2m.landsat.tm.c2.l2.scene.route.v1";
pub const LANDSAT_ETM_C2_L2_SCENE_ROUTE_ID: &str = "usgs.eros.m2m.landsat.etm.c2.l2.scene.route.v1";
pub const MPC_LANDSAT_SCENE_ROUTE_ID: &str = "planetary-computer.landsat-c2-l2.scene.route.v1";
pub const EARTH_SEARCH_LANDSAT_SCENE_ROUTE_ID: &str =
    "element84.earth-search.landsat-c2-l2.scene.route.v1";
pub const ALOS2_ROUTE_ID: &str = "asf.alos-2.palsar-2.scansar.l1.1.route.v1";
pub const NISAR_ROUTE_ID: &str = "asf.nisar.l.rslc.provisional.route.v1";
pub const BHOONIDHI_ROUTE_ID: &str = "bhoonidhi.nisar.s.ssar.rslc.route.v1";
pub const OPERA_S1_ROUTE_ID: &str = "route.asf.opera-s1.catalog.v1";
pub const LICSAR_ROUTE_ID: &str = "route.comet.licsar.catalog.v1";
pub const ASTER_GDEM_V3_ROUTE_ID: &str = "route.lpdaac.aster-gdem-v3.earthdata.v1";
pub const USGS_EARTHQUAKE_ROUTE_ID: &str = "route.usgs.earthquake.fdsn.v1";
pub const NGL_GNSS_ROUTE_ID: &str = "route.unr.ngl.geodesy.v1";
pub const ERA5_ROUTE_ID: &str = "route.era5.arco.v1";
pub const IGS_GIM_ROUTE_ID: &str = "route.cddis.ionex.v1";

const SENTINEL2_L2A_SUPPORTED_FILTER_KEYS: &[&str] =
    &["product_type", "cloud_cover_lte", "datetime", "spacecraft"];
const SENTINEL2_L1C_SUPPORTED_FILTER_KEYS: &[&str] =
    &["product_type", "cloud_cover_lte", "datetime", "spacecraft"];
const SENTINEL3_STANDARD_SUPPORTED_FILTER_KEYS: &[&str] =
    &["product_type", "cloud_cover_lte", "datetime", "spacecraft"];
const LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS: &[&str] = &[
    "cloud_cover_lte",
    "datetime",
    "spacecraft",
    "processing_levels",
];
const MPC_REVIEWED_COG_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime"];
const MPC_HLS_SUPPORTED_FILTER_KEYS: &[&str] = &["cloud_cover_lte", "datetime", "spacecraft"];
const ALOS2_SUPPORTED_FILTER_KEYS: &[&str] = &[
    "datetime",
    "beam_mode",
    "polarizations",
    "flight_direction",
    "look_direction",
    "path",
    "frame",
];
const NISAR_SUPPORTED_FILTER_KEYS: &[&str] = &[
    "datetime",
    "production_configuration",
    "frame_coverage",
    "frequency_a_polarization",
    "frequency_b_polarization",
    "range_bandwidth",
    "joint_observation",
    "flight_direction",
    "look_direction",
    "track",
    "relative_orbit",
    "frame",
];
const BHOONIDHI_SUPPORTED_FILTER_KEYS: &[&str] = &[
    "datetime",
    "online",
    "mode",
    "polarization",
    "acquisition_mode",
    "product_code",
    "software_version",
    "producer",
    "platform",
    "instrument",
    "scene_identifier",
];
const OPERA_S1_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime", "track", "frame", "flight_direction"];
const LICSAR_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime", "track", "frame"];
const ASTER_GDEM_V3_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime"];
const USGS_EARTHQUAKE_SUPPORTED_FILTER_KEYS: &[&str] =
    &["datetime", "min_magnitude", "max_magnitude", "max_depth_km"];
const NGL_GNSS_SUPPORTED_FILTER_KEYS: &[&str] = &["station_id", "max_distance_km"];
const ERA5_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime", "pressure_levels"];
const IGS_GIM_SUPPORTED_FILTER_KEYS: &[&str] = &["datetime"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatasetRoutePolicy {
    pub route_id: &'static str,
    pub logical_dataset: &'static str,
    pub provider_id: &'static str,
    pub native_collection_id: &'static str,
    pub catalogue_aoi_crs: &'static str,
    pub collection_profile_id: &'static str,
    pub production_enabled: bool,
    pub supported_filter_keys: &'static [&'static str],
}

const SENTINEL2_L2A_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: SENTINEL2_L2A_ROUTE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-2",
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: SENTINEL2_L2A_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: SENTINEL2_L2A_SUPPORTED_FILTER_KEYS,
};

const MPC_SENTINEL2_L2A_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: MPC_SENTINEL2_L2A_ROUTE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: MPC_SENTINEL2_L2A_PROVIDER_ID,
    native_collection_id: MPC_SENTINEL2_L2A_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: MPC_SENTINEL2_L2A_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: SENTINEL2_L2A_SUPPORTED_FILTER_KEYS,
};

const EARTH_SEARCH_SENTINEL2_L2A_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: EARTH_SEARCH_SENTINEL2_L2A_ROUTE_ID,
    logical_dataset: "optical.sentinel-2.l2a",
    provider_id: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    native_collection_id: EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: SENTINEL2_L2A_SUPPORTED_FILTER_KEYS,
};

const MPC_SENTINEL1_GRD_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: MPC_SENTINEL1_GRD_ROUTE_ID,
    logical_dataset: MPC_SENTINEL1_GRD_LOGICAL_DATASET,
    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
    native_collection_id: MPC_SENTINEL1_GRD_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: MPC_SENTINEL1_GRD_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: MPC_REVIEWED_COG_SUPPORTED_FILTER_KEYS,
};

const MPC_COPDEM_GLO30_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: MPC_COPDEM_GLO30_ROUTE_ID,
    logical_dataset: MPC_COPDEM_GLO30_LOGICAL_DATASET,
    provider_id: PLANETARY_COMPUTER_REVIEWED_PROVIDER_ID,
    native_collection_id: MPC_COPDEM_GLO30_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: MPC_COPDEM_GLO30_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: MPC_REVIEWED_COG_SUPPORTED_FILTER_KEYS,
};

const SENTINEL2_L1C_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: SENTINEL2_L1C_ROUTE_ID,
    logical_dataset: "optical.sentinel-2.l1c",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-2",
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: SENTINEL2_L1C_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: SENTINEL2_L1C_SUPPORTED_FILTER_KEYS,
};

const SENTINEL3_STANDARD_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: SENTINEL3_STANDARD_ROUTE_ID,
    logical_dataset: "optical.sentinel-3.standard",
    provider_id: "copernicus.dataspace.odata",
    native_collection_id: "SENTINEL-3",
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: SENTINEL3_STANDARD_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: SENTINEL3_STANDARD_SUPPORTED_FILTER_KEYS,
};

const LANDSAT_C2_L2_SCENE_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: LANDSAT_C2_L2_SCENE_ROUTE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: LANDSAT_C2_L2_PROVIDER_ID,
    native_collection_id: LANDSAT_C2_L2_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
    production_enabled: landsat_scene_search_live_accepted(),
    supported_filter_keys: LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS,
};

const LANDSAT_TM_C2_L2_SCENE_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: LANDSAT_TM_C2_L2_SCENE_ROUTE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: LANDSAT_C2_L2_PROVIDER_ID,
    native_collection_id: "landsat_tm_c2_l2",
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
    production_enabled: landsat_scene_search_live_accepted(),
    supported_filter_keys: LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS,
};

const LANDSAT_ETM_C2_L2_SCENE_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: LANDSAT_ETM_C2_L2_SCENE_ROUTE_ID,
    logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
    provider_id: LANDSAT_C2_L2_PROVIDER_ID,
    native_collection_id: "landsat_etm_c2_l2",
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: LANDSAT_C2_L2_SCENE_PROFILE_ID,
    production_enabled: landsat_scene_search_live_accepted(),
    supported_filter_keys: LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS,
};

const ALOS2_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: ALOS2_ROUTE_ID,
    logical_dataset: ALOS2_LOGICAL_DATASET,
    provider_id: ALOS2_PROVIDER_ID,
    native_collection_id: ALOS2_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: ALOS2_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: ALOS2_SUPPORTED_FILTER_KEYS,
};

const NISAR_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: NISAR_ROUTE_ID,
    logical_dataset: NISAR_LOGICAL_DATASET,
    provider_id: NISAR_PROVIDER_ID,
    native_collection_id: NISAR_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: NISAR_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: NISAR_SUPPORTED_FILTER_KEYS,
};

const BHOONIDHI_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: BHOONIDHI_ROUTE_ID,
    logical_dataset: BHOONIDHI_LOGICAL_DATASET,
    provider_id: BHOONIDHI_PROVIDER_ID,
    native_collection_id: BHOONIDHI_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: BHOONIDHI_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: BHOONIDHI_SUPPORTED_FILTER_KEYS,
};

const OPERA_S1_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: OPERA_S1_ROUTE_ID,
    logical_dataset: OPERA_S1_LOGICAL_DATASET,
    provider_id: OPERA_S1_PROVIDER_ID,
    native_collection_id: OPERA_S1_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: OPERA_S1_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: OPERA_S1_SUPPORTED_FILTER_KEYS,
};

const LICSAR_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: LICSAR_ROUTE_ID,
    logical_dataset: LICSAR_LOGICAL_DATASET,
    provider_id: LICSAR_PROVIDER_ID,
    native_collection_id: LICSAR_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: LICSAR_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: LICSAR_SUPPORTED_FILTER_KEYS,
};

const ASTER_GDEM_V3_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: ASTER_GDEM_V3_ROUTE_ID,
    logical_dataset: ASTER_GDEM_V3_LOGICAL_DATASET,
    provider_id: ASTER_GDEM_V3_PROVIDER_ID,
    native_collection_id: ASTER_GDEM_V3_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: ASTER_GDEM_V3_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: ASTER_GDEM_V3_SUPPORTED_FILTER_KEYS,
};

const USGS_EARTHQUAKE_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: USGS_EARTHQUAKE_ROUTE_ID,
    logical_dataset: USGS_EARTHQUAKE_LOGICAL_DATASET,
    provider_id: USGS_EARTHQUAKE_PROVIDER_ID,
    native_collection_id: USGS_EARTHQUAKE_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: USGS_EARTHQUAKE_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: USGS_EARTHQUAKE_SUPPORTED_FILTER_KEYS,
};

const NGL_GNSS_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: NGL_GNSS_ROUTE_ID,
    logical_dataset: NGL_GNSS_LOGICAL_DATASET,
    provider_id: NGL_GNSS_PROVIDER_ID,
    native_collection_id: NGL_GNSS_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: NGL_GNSS_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: NGL_GNSS_SUPPORTED_FILTER_KEYS,
};

const ERA5_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: ERA5_ROUTE_ID,
    logical_dataset: ERA5_LOGICAL_DATASET,
    provider_id: ERA5_PROVIDER_ID,
    native_collection_id: ERA5_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: ERA5_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: ERA5_SUPPORTED_FILTER_KEYS,
};

const IGS_GIM_ROUTE: DatasetRoutePolicy = DatasetRoutePolicy {
    route_id: IGS_GIM_ROUTE_ID,
    logical_dataset: IGS_GIM_LOGICAL_DATASET,
    provider_id: IGS_GIM_PROVIDER_ID,
    native_collection_id: IGS_GIM_NATIVE_COLLECTION_ID,
    catalogue_aoi_crs: "EPSG:4326",
    collection_profile_id: IGS_GIM_PROFILE_ID,
    production_enabled: true,
    supported_filter_keys: IGS_GIM_SUPPORTED_FILTER_KEYS,
};

const ROUTES: &[DatasetRoutePolicy] = &[
    SENTINEL2_L2A_ROUTE,
    MPC_SENTINEL2_L2A_ROUTE,
    EARTH_SEARCH_SENTINEL2_L2A_ROUTE,
    MPC_SENTINEL1_GRD_ROUTE,
    MPC_COPDEM_GLO30_ROUTE,
    DatasetRoutePolicy {
        route_id: MPC_HLS_L30_ROUTE_ID,
        logical_dataset: MPC_HLS_L30_LOGICAL_DATASET,
        provider_id: MPC_HLS_PROVIDER_ID,
        native_collection_id: MPC_HLS_L30_COLLECTION_ID,
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: MPC_HLS_L30_PROFILE_ID,
        production_enabled: true,
        supported_filter_keys: MPC_HLS_SUPPORTED_FILTER_KEYS,
    },
    DatasetRoutePolicy {
        route_id: MPC_HLS_S30_ROUTE_ID,
        logical_dataset: MPC_HLS_S30_LOGICAL_DATASET,
        provider_id: MPC_HLS_PROVIDER_ID,
        native_collection_id: MPC_HLS_S30_COLLECTION_ID,
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: MPC_HLS_S30_PROFILE_ID,
        production_enabled: true,
        supported_filter_keys: MPC_HLS_SUPPORTED_FILTER_KEYS,
    },
    SENTINEL2_L1C_ROUTE,
    SENTINEL3_STANDARD_ROUTE,
    LANDSAT_C2_L2_SCENE_ROUTE,
    LANDSAT_TM_C2_L2_SCENE_ROUTE,
    LANDSAT_ETM_C2_L2_SCENE_ROUTE,
    DatasetRoutePolicy {
        route_id: MPC_LANDSAT_SCENE_ROUTE_ID,
        logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
        provider_id: MPC_LANDSAT_PROVIDER_ID,
        native_collection_id: MPC_LANDSAT_COLLECTION_ID,
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: MPC_LANDSAT_SCENE_PROFILE_ID,
        production_enabled: true,
        supported_filter_keys: LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS,
    },
    DatasetRoutePolicy {
        route_id: EARTH_SEARCH_LANDSAT_SCENE_ROUTE_ID,
        logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
        provider_id: EARTH_SEARCH_LANDSAT_PROVIDER_ID,
        native_collection_id: EARTH_SEARCH_LANDSAT_COLLECTION_ID,
        catalogue_aoi_crs: "EPSG:4326",
        collection_profile_id: EARTH_SEARCH_LANDSAT_SCENE_PROFILE_ID,
        production_enabled: true,
        supported_filter_keys: LANDSAT_C2_L2_SUPPORTED_FILTER_KEYS,
    },
    ALOS2_ROUTE,
    NISAR_ROUTE,
    BHOONIDHI_ROUTE,
    OPERA_S1_ROUTE,
    LICSAR_ROUTE,
    ASTER_GDEM_V3_ROUTE,
    USGS_EARTHQUAKE_ROUTE,
    NGL_GNSS_ROUTE,
    ERA5_ROUTE,
    IGS_GIM_ROUTE,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteUse {
    OfflineFixture,
    Production,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatasetRoutePolicyError {
    UnknownLogicalDataset(String),
    DuplicateRouteId(String),
    DuplicateProviderCollection {
        logical_dataset: String,
        provider_id: String,
        native_collection_id: String,
    },
    ProviderMismatch {
        expected: &'static str,
        actual: String,
    },
    ProviderUnavailable(String),
    NativeCollectionMismatch {
        expected: &'static str,
        actual: String,
    },
    ProductionDisabled(&'static str),
    UnsupportedFilter(String),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DatasetRoutePolicyRegistry;

impl DatasetRoutePolicyRegistry {
    pub fn validate_definitions(
        definitions: &[DatasetRoutePolicy],
    ) -> Result<(), DatasetRoutePolicyError> {
        for (index, definition) in definitions.iter().enumerate() {
            for other in &definitions[index + 1..] {
                if other.route_id == definition.route_id {
                    return Err(DatasetRoutePolicyError::DuplicateRouteId(
                        definition.route_id.to_string(),
                    ));
                }
                if other.logical_dataset == definition.logical_dataset
                    && other.provider_id == definition.provider_id
                    && other.native_collection_id == definition.native_collection_id
                {
                    return Err(DatasetRoutePolicyError::DuplicateProviderCollection {
                        logical_dataset: definition.logical_dataset.to_string(),
                        provider_id: definition.provider_id.to_string(),
                        native_collection_id: definition.native_collection_id.to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn resolve_from_definitions<'a>(
        definitions: &'a [DatasetRoutePolicy],
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
        use_case: RouteUse,
    ) -> Result<&'a DatasetRoutePolicy, DatasetRoutePolicyError> {
        let dataset_routes = definitions
            .iter()
            .filter(|route| route.logical_dataset == logical_dataset)
            .collect::<Vec<_>>();
        if dataset_routes.is_empty() {
            return Err(DatasetRoutePolicyError::UnknownLogicalDataset(
                logical_dataset.to_string(),
            ));
        }

        if let Some(route) = dataset_routes.iter().copied().find(|route| {
            route.provider_id == provider_id && route.native_collection_id == native_collection_id
        }) {
            if matches!(use_case, RouteUse::Production) && !route.production_enabled {
                return Err(DatasetRoutePolicyError::ProductionDisabled(route.route_id));
            }
            return Ok(route);
        }

        let provider_routes = dataset_routes
            .iter()
            .copied()
            .filter(|route| route.provider_id == provider_id)
            .collect::<Vec<_>>();
        if provider_routes.is_empty() {
            if dataset_routes.len() == 1 {
                return Err(DatasetRoutePolicyError::ProviderMismatch {
                    expected: dataset_routes[0].provider_id,
                    actual: provider_id.to_string(),
                });
            }
            return Err(DatasetRoutePolicyError::ProviderUnavailable(
                provider_id.to_string(),
            ));
        }
        if provider_routes.len() == 1 {
            return Err(DatasetRoutePolicyError::NativeCollectionMismatch {
                expected: provider_routes[0].native_collection_id,
                actual: native_collection_id.to_string(),
            });
        }
        Err(DatasetRoutePolicyError::NativeCollectionMismatch {
            expected: provider_routes[0].native_collection_id,
            actual: native_collection_id.to_string(),
        })
    }

    pub fn resolve(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
        use_case: RouteUse,
    ) -> Result<&'static DatasetRoutePolicy, DatasetRoutePolicyError> {
        Self::resolve_from_definitions(
            ROUTES,
            logical_dataset,
            provider_id,
            native_collection_id,
            use_case,
        )
    }

    /// Return the legacy single route definition without applying runtime
    /// enablement. Once a logical dataset owns more than one route this method
    /// intentionally returns `None`, forcing multi-provider callers to use an
    /// exact provider/collection identity rather than silently choosing by list
    /// order.
    pub fn for_logical_dataset(
        &self,
        logical_dataset: &str,
    ) -> Option<&'static DatasetRoutePolicy> {
        let mut routes = ROUTES
            .iter()
            .filter(|route| route.logical_dataset == logical_dataset);
        let route = routes.next()?;
        routes.next().is_none().then_some(route)
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    #[doc(hidden)]
    pub fn resolve_nisar_live_acceptance_candidate(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Result<&'static DatasetRoutePolicy, DatasetRoutePolicyError> {
        let route = self.resolve(
            logical_dataset,
            provider_id,
            native_collection_id,
            RouteUse::OfflineFixture,
        )?;
        if route.route_id != NISAR_ROUTE_ID {
            return Err(DatasetRoutePolicyError::ProductionDisabled(route.route_id));
        }
        Ok(route)
    }

    #[cfg(feature = "usgs-m2m-live-acceptance")]
    #[doc(hidden)]
    pub fn resolve_live_acceptance_candidate(
        &self,
        logical_dataset: &str,
        provider_id: &str,
        native_collection_id: &str,
    ) -> Result<&'static DatasetRoutePolicy, DatasetRoutePolicyError> {
        let route = self.resolve(
            logical_dataset,
            provider_id,
            native_collection_id,
            RouteUse::OfflineFixture,
        )?;
        if route.route_id != LANDSAT_C2_L2_SCENE_ROUTE_ID {
            return Err(DatasetRoutePolicyError::ProductionDisabled(route.route_id));
        }
        Ok(route)
    }

    pub fn validate_filter_keys(
        &self,
        logical_dataset: &str,
        filter_keys: &[&str],
    ) -> Result<(), DatasetRoutePolicyError> {
        let route = ROUTES
            .iter()
            .find(|route| route.logical_dataset == logical_dataset)
            .ok_or_else(|| {
                DatasetRoutePolicyError::UnknownLogicalDataset(logical_dataset.to_string())
            })?;
        for filter_key in filter_keys {
            if !route.supported_filter_keys.contains(filter_key) {
                return Err(DatasetRoutePolicyError::UnsupportedFilter(
                    (*filter_key).to_string(),
                ));
            }
        }
        Ok(())
    }
}
