use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OFFICIAL_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/samapriya/Earth-Engine-Datasets-List/master/gee_catalog.json";
const COMMUNITY_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/samapriya/awesome-gee-community-datasets/master/community_datasets.json";
const CACHE_TTL_SECS: u64 = 7 * 24 * 3600;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeeDatasetProfile {
    pub id: String,
    pub title: String,
    pub title_zh: String,
    pub source: String,     // "official" | "community"
    pub asset_type: String, // "image_collection" | "image" | "table"
    pub provider: String,
    pub start_date: String,
    pub end_date: String,
    pub native_scale_m: f64,
    pub cloud_property: Option<String>,
    pub default_bands: Vec<String>,
    pub vis_min: f64,
    pub vis_max: f64,
    pub tags: String,
    pub asset_url: String,
}

pub fn builtin_curated_datasets() -> Vec<GeeDatasetProfile> {
    vec![
        GeeDatasetProfile {
            id: "COPERNICUS/S2_SR_HARMONIZED".into(),
            title: "Harmonized Sentinel-2 MSI: MultiSpectral Instrument, Level-2A".into(),
            title_zh: "哨兵2号 (Sentinel-2) 10米多光谱地表反射率 L2A".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "European Union/ESA/Copernicus".into(),
            start_date: "2017-03-28".into(),
            end_date: "present".into(),
            native_scale_m: 10.0,
            cloud_property: Some("CLOUDY_PIXEL_PERCENTAGE".into()),
            default_bands: vec!["B4".into(), "B3".into(), "B2".into()],
            vis_min: 0.0,
            vis_max: 3000.0,
            tags: "sentinel-2 s2 optical multispectral reflectance ndvi ndwi mndwi ndbi nbr lulc 哨兵2 光学 地表反射率 植被 水体".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S2_SR_HARMONIZED".into(),
        },
        GeeDatasetProfile {
            id: "COPERNICUS/S1_GRD".into(),
            title: "Sentinel-1 SAR GRD: C-band Synthetic Aperture Radar Ground Range Detected".into(),
            title_zh: "哨兵1号 (Sentinel-1) 10米 C波段合成孔径雷达 SAR GRD".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "European Union/ESA/Copernicus".into(),
            start_date: "2014-10-03".into(),
            end_date: "present".into(),
            native_scale_m: 10.0,
            cloud_property: None,
            default_bands: vec!["VV".into(), "VH".into()],
            vis_min: -25.0,
            vis_max: 0.0,
            tags: "sentinel-1 s1 sar radar c-band backscatter flood water vv vh 哨兵1 雷达 合成孔径 洪水 极化".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S1_GRD".into(),
        },
        GeeDatasetProfile {
            id: "LANDSAT/LC09/C02/T1_L2".into(),
            title: "USGS Landsat 9 Level 2, Collection 2, Tier 1".into(),
            title_zh: "陆地卫星9号 (Landsat 9) 30米地表反射率与地表温度 C2-L2".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "USGS".into(),
            start_date: "2021-10-31".into(),
            end_date: "present".into(),
            native_scale_m: 30.0,
            cloud_property: Some("CLOUD_COVER".into()),
            default_bands: vec!["SR_B4".into(), "SR_B3".into(), "SR_B2".into()],
            vis_min: 7000.0,
            vis_max: 16000.0,
            tags: "landsat-9 lc09 optical thermal lst surface reflectance 陆地卫星9 光学 地表温度 热红外".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC09_C02_T1_L2".into(),
        },
        GeeDatasetProfile {
            id: "LANDSAT/LC08/C02/T1_L2".into(),
            title: "USGS Landsat 8 Level 2, Collection 2, Tier 1".into(),
            title_zh: "陆地卫星8号 (Landsat 8) 30米地表反射率与地表温度 C2-L2".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "USGS".into(),
            start_date: "2013-03-18".into(),
            end_date: "present".into(),
            native_scale_m: 30.0,
            cloud_property: Some("CLOUD_COVER".into()),
            default_bands: vec!["SR_B4".into(), "SR_B3".into(), "SR_B2".into()],
            vis_min: 7000.0,
            vis_max: 16000.0,
            tags: "landsat-8 lc08 optical thermal lst surface reflectance 陆地卫星8 光学 地表温度 热红外".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LC08_C02_T1_L2".into(),
        },
        GeeDatasetProfile {
            id: "LANDSAT/LT05/C02/T1_L2".into(),
            title: "USGS Landsat 5 TM Collection 2 Tier 1 Level-2".into(),
            title_zh: "陆地卫星5号 (Landsat 5 TM) 1984-2012 历史长时序地表反射率".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "USGS".into(),
            start_date: "1984-03-16".into(),
            end_date: "2012-05-05".into(),
            native_scale_m: 30.0,
            cloud_property: Some("CLOUD_COVER".into()),
            default_bands: vec!["SR_B3".into(), "SR_B2".into(), "SR_B1".into()],
            vis_min: 7000.0,
            vis_max: 16000.0,
            tags: "landsat-5 lt05 historical archive optical 历史影像 长时序 陆地卫星5".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/LANDSAT_LT05_C02_T1_L2".into(),
        },
        GeeDatasetProfile {
            id: "COPERNICUS/DEM/GLO30".into(),
            title: "Copernicus DEM GLO-30: Global 30m Digital Elevation Model".into(),
            title_zh: "哥白尼全球 30米数字高程模型 (Copernicus DEM GLO-30)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "European Space Agency / Copernicus".into(),
            start_date: "2010-01-01".into(),
            end_date: "2015-01-01".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["DEM".into()],
            vis_min: 0.0,
            vis_max: 2000.0,
            tags: "dem elevation terrain topography glo30 copernicus 高程 地形 哥白尼 数字高程".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_DEM_GLO30".into(),
        },
        GeeDatasetProfile {
            id: "NASA/NASADEM_HGT/001".into(),
            title: "NASADEM: NASA NASADEM Digital Elevation 30m".into(),
            title_zh: "NASA 全球 30米数字高程模型 (NASADEM)".into(),
            source: "official".into(),
            asset_type: "image".into(),
            provider: "NASA / JPL".into(),
            start_date: "2000-02-11".into(),
            end_date: "2000-02-22".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["elevation".into()],
            vis_min: 0.0,
            vis_max: 2000.0,
            tags: "dem elevation nasadem srtm terrain 高程 地形".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_NASADEM_HGT_001".into(),
        },
        GeeDatasetProfile {
            id: "USGS/SRTMGL1_003".into(),
            title: "NASA SRTM Digital Elevation 30m".into(),
            title_zh: "SRTM 全球 30米数字高程模型 (SRTMGL1)".into(),
            source: "official".into(),
            asset_type: "image".into(),
            provider: "NASA / USGS".into(),
            start_date: "2000-02-11".into(),
            end_date: "2000-02-22".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["elevation".into()],
            vis_min: 0.0,
            vis_max: 2000.0,
            tags: "srtm dem elevation terrain topography 高程 地形".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/USGS_SRTMGL1_003".into(),
        },
        GeeDatasetProfile {
            id: "projects/climate-and-sustainability/assets/gem-15_v1".into(),
            title: "Google GEM-15: 15m Global Elevation Model v1 (DSM & Bare-Earth DTM)".into(),
            title_zh: "Google GEM-15 近全球 15米数字高程模型 v1 (裸地 DTM + 地表 DSM · 不含中国)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "Google Climate and Sustainability · CC-BY-4.0".into(),
            start_date: "2018-01-01".into(),
            end_date: "2025-12-31".into(),
            native_scale_m: 15.0,
            cloud_property: None,
            default_bands: vec!["dtm".into(), "dsm".into()],
            vis_min: 0.0,
            vis_max: 2000.0,
            tags: "gem-15 gem15 google 15m dem dtm dsm bare-earth elevation terrain egm96 cc-by-4.0 高程 地形 裸地 15米 海外 不含中国".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/projects_climate-and-sustainability_assets_gem-15_v1".into(),
        },
        GeeDatasetProfile {
            id: "GOOGLE/DYNAMICWORLD/V1".into(),
            title: "Dynamic World V1 — Near Real-Time 10m Land Use Land Cover".into(),
            title_zh: "谷歌 Dynamic World 10米准实时全球土地覆盖分类".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "Google / World Resources Institute".into(),
            start_date: "2015-06-27".into(),
            end_date: "present".into(),
            native_scale_m: 10.0,
            cloud_property: None,
            default_bands: vec!["label".into()],
            vis_min: 0.0,
            vis_max: 8.0,
            tags: "landcover lulc dynamic world classification 土地覆盖 土地利用 地物分类 建成区 水体 森林".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_DYNAMICWORLD_V1".into(),
        },
        GeeDatasetProfile {
            id: "ESA/WorldCover/v200".into(),
            title: "ESA WorldCover 10m v200 Global Land Cover".into(),
            title_zh: "欧空局 WorldCover 10米全球精细土地覆盖 (v200)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "ESA".into(),
            start_date: "2021-01-01".into(),
            end_date: "2022-01-01".into(),
            native_scale_m: 10.0,
            cloud_property: None,
            default_bands: vec!["Map".into()],
            vis_min: 10.0,
            vis_max: 100.0,
            tags: "esa worldcover landcover lulc 10m 欧空局 土地覆盖 土地利用 地物分类".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/ESA_WorldCover_v200".into(),
        },
        GeeDatasetProfile {
            id: "JRC/GSW1_4/GlobalSurfaceWater".into(),
            title: "JRC Global Surface Water Mapping Layers, v1.4 (30m)".into(),
            title_zh: "JRC 全球 30米地表水体长期频率与动态变化分布".into(),
            source: "official".into(),
            asset_type: "image".into(),
            provider: "EC JRC / Google".into(),
            start_date: "1984-03-16".into(),
            end_date: "2021-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["occurrence".into()],
            vis_min: 0.0,
            vis_max: 100.0,
            tags: "water surface hydrology flood lake river jrc gsw 水体 湖泊 河流 洪水 淹没".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/JRC_GSW1_4_GlobalSurfaceWater".into(),
        },
        GeeDatasetProfile {
            id: "JRC/GSW1_4/MonthlyHistory".into(),
            title: "JRC Global Surface Water Monthly History, v1.4 (30m)".into(),
            title_zh: "JRC 全球 30米地表水体月度历史变化".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "EC JRC / Google".into(),
            start_date: "1984-03-16".into(),
            end_date: "2021-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["water".into()],
            vis_min: 0.0,
            vis_max: 2.0,
            tags: "water monthly history flood jrc gsw 月度水体 洪水演变".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/JRC_GSW1_4_MonthlyHistory".into(),
        },
        GeeDatasetProfile {
            id: "UMD/hansen/global_forest_change_2023_v1_11".into(),
            title: "Hansen Global Forest Change v1.11 (2000-2023)".into(),
            title_zh: "Hansen 全球 30米森林损毁与树冠覆盖变化 (2000-2023)".into(),
            source: "official".into(),
            asset_type: "image".into(),
            provider: "UMD / Google".into(),
            start_date: "2000-01-01".into(),
            end_date: "2023-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["treecover2000".into(), "loss".into(), "lossyear".into()],
            vis_min: 0.0,
            vis_max: 100.0,
            tags: "forest loss gain hansen umd 森林 砍伐 森林损毁".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/UMD_hansen_global_forest_change_2023_v1_11".into(),
        },
        GeeDatasetProfile {
            id: "NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG".into(),
            title: "VIIRS Stray Light Corrected Nighttime Day/Night Band Composites".into(),
            title_zh: "VIIRS 全球月度夜间灯光遥感合成数据 (约500米)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NOAA / Earth Observation Group".into(),
            start_date: "2014-01-01".into(),
            end_date: "present".into(),
            native_scale_m: 463.0,
            cloud_property: None,
            default_bands: vec!["avg_rad".into()],
            vis_min: 0.0,
            vis_max: 60.0,
            tags: "viirs dnb nighttime lights urban economy 夜间灯光 夜光 城市扩张 经济活力".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NOAA_VIIRS_DNB_MONTHLY_V1_VCMSLCFG".into(),
        },
        GeeDatasetProfile {
            id: "MODIS/061/MOD11A1".into(),
            title: "MOD11A1.061 Terra Land Surface Temperature and Emissivity Daily Global 1km".into(),
            title_zh: "MODIS 全球逐日 1公里地表温度 (LST 白天·夜间)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA LP DAAC".into(),
            start_date: "2000-02-24".into(),
            end_date: "present".into(),
            native_scale_m: 1000.0,
            cloud_property: None,
            default_bands: vec!["LST_Day_1km".into()],
            vis_min: 13000.0,
            vis_max: 16500.0,
            tags: "modis lst temperature thermal heat island 地表温度 热岛效应 热红外 温度".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD11A1".into(),
        },
        GeeDatasetProfile {
            id: "MODIS/061/MOD13Q1".into(),
            title: "MOD13Q1.061 Terra Vegetation Indices 16-Day Global 250m".into(),
            title_zh: "MODIS 全球 250米 16天合成植被指数 (NDVI · EVI)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA LP DAAC".into(),
            start_date: "2000-02-18".into(),
            end_date: "present".into(),
            native_scale_m: 250.0,
            cloud_property: None,
            default_bands: vec!["NDVI".into(), "EVI".into()],
            vis_min: 0.0,
            vis_max: 9000.0,
            tags: "modis ndvi evi vegetation greenness 植被指数 绿度 生态 农情".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MOD13Q1".into(),
        },
        GeeDatasetProfile {
            id: "UCSB-CHG/CHIRPS/DAILY".into(),
            title: "CHIRPS Daily: Climate Hazards Group InfraRed Precipitation With Station Data".into(),
            title_zh: "CHIRPS 全球逐日降雨量高分辨率数据集 (约5.5公里)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "UCSB / CHG".into(),
            start_date: "1981-01-01".into(),
            end_date: "present".into(),
            native_scale_m: 5566.0,
            cloud_property: None,
            default_bands: vec!["precipitation".into()],
            vis_min: 0.0,
            vis_max: 50.0,
            tags: "chirps precipitation rainfall drought climate 降水 降雨 暴雨 干旱 气象".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/UCSB-CHG_CHIRPS_DAILY".into(),
        },
        GeeDatasetProfile {
            id: "NASA/GPM_L3/IMERG_V07".into(),
            title: "GPM: Global Precipitation Measurement (IMERG) v07 Half-Hourly".into(),
            title_zh: "NASA GPM IMERG 全球半小时卫星降水监测数据 (v07)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA GES DISC".into(),
            start_date: "2000-06-01".into(),
            end_date: "present".into(),
            native_scale_m: 11132.0,
            cloud_property: None,
            default_bands: vec!["precipitation".into()],
            vis_min: 0.0,
            vis_max: 20.0,
            tags: "gpm imerg precipitation rain storm typhoon 降水 暴雨 台风 降雨".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_GPM_L3_IMERG_V07".into(),
        },
        GeeDatasetProfile {
            id: "ECMWF/ERA5_LAND/DAILY_AGGR".into(),
            title: "ERA5-Land Daily Aggregated - ECMWF Climate Reanalysis".into(),
            title_zh: "ECMWF ERA5-Land 全球逐日陆面气象再分析 (气温·降水·土壤湿度)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "Copernicus / ECMWF".into(),
            start_date: "1950-01-02".into(),
            end_date: "present".into(),
            native_scale_m: 11132.0,
            cloud_property: None,
            default_bands: vec!["temperature_2m".into(), "total_precipitation_sum".into()],
            vis_min: 250.0,
            vis_max: 315.0,
            tags: "era5 climate reanalysis temperature wind soil moisture precipitation 气象 再分析 气温 土壤湿度 降水".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/ECMWF_ERA5_LAND_DAILY_AGGR".into(),
        },
        GeeDatasetProfile {
            id: "ECMWF/ERA5_LAND/HOURLY".into(),
            title: "ERA5-Land Hourly - ECMWF Climate Reanalysis".into(),
            title_zh: "ECMWF ERA5-Land 全球逐小时陆面气象再分析".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "Copernicus / ECMWF".into(),
            start_date: "1950-01-01".into(),
            end_date: "present".into(),
            native_scale_m: 11132.0,
            cloud_property: None,
            default_bands: vec!["temperature_2m".into()],
            vis_min: 250.0,
            vis_max: 315.0,
            tags: "era5 hourly climate weather temperature 逐小时 气象 再分析".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/ECMWF_ERA5_LAND_HOURLY".into(),
        },
        GeeDatasetProfile {
            id: "NASA/SMAP/SPL4SMGP/007".into(),
            title: "SMAP L4 Global 3-hourly 9-km Surface and Root Zone Soil Moisture".into(),
            title_zh: "NASA SMAP 全球 9公里表层与根区土壤湿度".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA NSIDC".into(),
            start_date: "2015-03-31".into(),
            end_date: "present".into(),
            native_scale_m: 9000.0,
            cloud_property: None,
            default_bands: vec!["sm_surface".into(), "sm_rootzone".into()],
            vis_min: 0.0,
            vis_max: 0.5,
            tags: "smap soil moisture drought agriculture 土壤湿度 墒情 干旱 农业".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_SMAP_SPL4SMGP_007".into(),
        },
        GeeDatasetProfile {
            id: "COPERNICUS/S5P/OFFL/L3_NO2".into(),
            title: "Sentinel-5P OFFL NO2: Offline Nitrogen Dioxide".into(),
            title_zh: "哨兵5P (Sentinel-5P) 大气二氧化氮柱浓度监测".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "European Union/ESA/Copernicus".into(),
            start_date: "2018-06-28".into(),
            end_date: "present".into(),
            native_scale_m: 1113.2,
            cloud_property: None,
            default_bands: vec!["tropospheric_NO2_column_number_density".into()],
            vis_min: 0.0,
            vis_max: 0.0002,
            tags: "sentinel-5p s5p no2 air quality pollution atmosphere 二氧化氮 大气污染 空气质量".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_S5P_OFFL_L3_NO2".into(),
        },
        GeeDatasetProfile {
            id: "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH".into(),
            title: "Global PALSAR-2/PALSAR Yearly Mosaic (L-band SAR 25m)".into(),
            title_zh: "ALOS PALSAR-2 全球 25米 L波段雷达年度拼图".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "JAXA EORC".into(),
            start_date: "2007-01-01".into(),
            end_date: "present".into(),
            native_scale_m: 25.0,
            cloud_property: None,
            default_bands: vec!["HH".into(), "HV".into()],
            vis_min: 0.0,
            vis_max: 10000.0,
            tags: "alos palsar l-band sar radar forest biomass L波段 雷达 森林 生物量".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/JAXA_ALOS_PALSAR_YEARLY_SAR_EPOCH".into(),
        },
        GeeDatasetProfile {
            id: "LARSE/GEDI/GEDI02_A_002_MONTHLY".into(),
            title: "GEDI L2A Raster Canopy Top Height (Version 2) 25m".into(),
            title_zh: "GEDI 星载激光雷达全球 25米森林冠层高度 (月度)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "USFS / NASA GEDI".into(),
            start_date: "2019-04-01".into(),
            end_date: "present".into(),
            native_scale_m: 25.0,
            cloud_property: None,
            default_bands: vec!["rh98".into()],
            vis_min: 1.0,
            vis_max: 40.0,
            tags: "gedi lidar canopy height forest tree biomass 树高 冠层高度 森林 激光雷达".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/LARSE_GEDI_GEDI02_A_002_MONTHLY".into(),
        },
        GeeDatasetProfile {
            id: "WorldPop/GP/100m/pop".into(),
            title: "WorldPop Global Project Population Data: Estimated Residential Population per 100x100m".into(),
            title_zh: "WorldPop 全球 100米高分辨率人口空间分布网格".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "WorldPop".into(),
            start_date: "2000-01-01".into(),
            end_date: "2021-01-01".into(),
            native_scale_m: 100.0,
            cloud_property: None,
            default_bands: vec!["population".into()],
            vis_min: 0.0,
            vis_max: 50.0,
            tags: "worldpop population demographic exposure 人口 人口密度 人口分布".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/WorldPop_GP_100m_pop".into(),
        },
        GeeDatasetProfile {
            id: "FIRMS".into(),
            title: "FIRMS: Fire Information for Resource Management System".into(),
            title_zh: "NASA FIRMS 全球近实时火点与火灾热异常监测".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA / LANCE".into(),
            start_date: "2000-11-01".into(),
            end_date: "present".into(),
            native_scale_m: 1000.0,
            cloud_property: None,
            default_bands: vec!["T21".into()],
            vis_min: 300.0,
            vis_max: 400.0,
            tags: "firms fire wildfire thermal anomaly 火灾 火点 山火 火烧".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/FIRMS".into(),
        },
        // Curated Community Catalog Flagships (awesome-gee-community-datasets)
        GeeDatasetProfile {
            id: "projects/sat-io/open-datasets/FABDEM".into(),
            title: "FABDEM: Forest And Buildings removed Copernicus 30m DEM".into(),
            title_zh: "FABDEM 剔除建筑与树冠偏差的全球 30米高精度裸地高程 (社区精选)".into(),
            source: "community".into(),
            asset_type: "image_collection".into(),
            provider: "University of Bristol / awesome-gee-community-catalog".into(),
            start_date: "2010-01-01".into(),
            end_date: "2018-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["b1".into()],
            vis_min: 0.0,
            vis_max: 1500.0,
            tags: "fabdem bare-earth dem elevation flood terrain 裸地高程 去除建筑森林 高程 社区".into(),
            asset_url: "https://gee-community-catalog.org/projects/fabdem/".into(),
        },
        GeeDatasetProfile {
            id: "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M".into(),
            title: "GHSL Global Building Height 100m (JRC)".into(),
            title_zh: "GHSL 全球城市建筑高度空间分布 (社区精选)".into(),
            source: "community".into(),
            asset_type: "image".into(),
            provider: "EC JRC / awesome-gee-community-catalog".into(),
            start_date: "2018-01-01".into(),
            end_date: "2018-12-31".into(),
            native_scale_m: 100.0,
            cloud_property: None,
            default_bands: vec!["b1".into()],
            vis_min: 0.0,
            vis_max: 50.0,
            tags: "building height urban ghsl morphology 建筑高度 城市三维 建成区 社区".into(),
            asset_url: "https://gee-community-catalog.org/projects/ghsl/".into(),
        },
        GeeDatasetProfile {
            id: "projects/sat-io/open-datasets/CLCD".into(),
            title: "China Land Cover Dataset (CLCD) Annual 30m (1985-2023)".into(),
            title_zh: "武汉大学中国逐年 30米土地覆盖数据集 CLCD (社区精选)".into(),
            source: "community".into(),
            asset_type: "image_collection".into(),
            provider: "Wuhan University / awesome-gee-community-catalog".into(),
            start_date: "1985-01-01".into(),
            end_date: "2023-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["b1".into()],
            vis_min: 1.0,
            vis_max: 9.0,
            tags: "china land cover clCD 30m annual 中国土地覆盖 武汉大学 土地利用 社区".into(),
            asset_url: "https://gee-community-catalog.org/projects/clcd/".into(),
        },
        GeeDatasetProfile {
            id: "COPERNICUS/DEM/GLO30_2024_1".into(),
            title: "Copernicus DEM GLO-30 (2024 Release)".into(),
            title_zh: "哥白尼全球 30米数字高程模型 (2024新版)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "European Space Agency / Copernicus".into(),
            start_date: "2024-01-01".into(),
            end_date: "2024-12-31".into(),
            native_scale_m: 30.0,
            cloud_property: None,
            default_bands: vec!["DEM".into()],
            vis_min: 0.0,
            vis_max: 2000.0,
            tags: "dem elevation terrain topography glo30 2024 copernicus 高程 地形 哥白尼 数字高程".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/COPERNICUS_DEM_GLO30_2024_1".into(),
        },
        GeeDatasetProfile {
            id: "NASA/SMAP/SPL4SMGP/008".into(),
            title: "SMAP L4 Global 3-hourly 9-km Surface and Root Zone Soil Moisture v008".into(),
            title_zh: "NASA SMAP L4 全球 9公里表层与根区土壤湿度 (v008)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA NSIDC".into(),
            start_date: "2015-03-31".into(),
            end_date: "present".into(),
            native_scale_m: 9000.0,
            cloud_property: None,
            default_bands: vec!["sm_surface".into(), "sm_rootzone".into()],
            vis_min: 0.0,
            vis_max: 0.5,
            tags: "smap soil moisture drought agriculture v008 土壤湿度 墒情 干旱 农业".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_SMAP_SPL4SMGP_008".into(),
        },
        GeeDatasetProfile {
            id: "JRC/GHSL/P2023A/GHS_POP".into(),
            title: "GHSL Global Population Surfaces P2023A (100m)".into(),
            title_zh: "JRC GHSL 全球 100米多期人口分布网格 (P2023A)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "EC JRC".into(),
            start_date: "1975-01-01".into(),
            end_date: "2030-01-01".into(),
            native_scale_m: 100.0,
            cloud_property: None,
            default_bands: vec!["population_count".into()],
            vis_min: 0.0,
            vis_max: 100.0,
            tags: "ghsl population exposure demographic 人口 人口分布 暴露".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/JRC_GHSL_P2023A_GHS_POP".into(),
        },
        GeeDatasetProfile {
            id: "GOOGLE/Research/open-buildings/v3/polygons".into(),
            title: "Google Open Buildings V3 Polygons".into(),
            title_zh: "Google 开放建筑物矢量轮廓数据 (V3)".into(),
            source: "official".into(),
            asset_type: "table".into(),
            provider: "Google Research".into(),
            start_date: "2023-05-01".into(),
            end_date: "present".into(),
            native_scale_m: 1.0,
            cloud_property: None,
            default_bands: vec!["confidence".into(), "area_in_meters".into()],
            vis_min: 0.5,
            vis_max: 1.0,
            tags: "open buildings footprint polygon exposure urban 建筑 建筑物 房屋 暴露".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/GOOGLE_Research_open-buildings_v3_polygons".into(),
        },
        GeeDatasetProfile {
            id: "MODIS/061/MCD64A1".into(),
            title: "MCD64A1.061 MODIS Burned Area Monthly Global 500m".into(),
            title_zh: "MODIS 全球逐月 500米火烧迹地产品 (MCD64A1)".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA LP DAAC".into(),
            start_date: "2000-11-01".into(),
            end_date: "present".into(),
            native_scale_m: 500.0,
            cloud_property: None,
            default_bands: vec!["BurnDate".into()],
            vis_min: 1.0,
            vis_max: 366.0,
            tags: "modis mcd64a1 burned area wildfire fire 火烧迹地 火灾 野火 过火区".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/MODIS_061_MCD64A1".into(),
        },
        GeeDatasetProfile {
            id: "NASA/HLS/HLSL30/v002".into(),
            title: "NASA HLS L30 Landsat-8/9 Harmonized 30m Surface Reflectance".into(),
            title_zh: "NASA HLS L30 融合陆地卫星 30米一致化地表反射率".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA LP DAAC".into(),
            start_date: "2013-04-11".into(),
            end_date: "present".into(),
            native_scale_m: 30.0,
            cloud_property: Some("CLOUD_COVERAGE".into()),
            default_bands: vec!["B4".into(), "B3".into(), "B2".into()],
            vis_min: 0.0,
            vis_max: 0.3,
            tags: "hls hlsl30 landsat harmonized surface reflectance optical 跨传感器一致化 光学 地表反射率".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_HLS_HLSL30_v002".into(),
        },
        GeeDatasetProfile {
            id: "NASA/HLS/HLSS30/v002".into(),
            title: "NASA HLS S30 Sentinel-2 Harmonized 30m Surface Reflectance".into(),
            title_zh: "NASA HLS S30 融合哨兵2号 30米一致化地表反射率".into(),
            source: "official".into(),
            asset_type: "image_collection".into(),
            provider: "NASA LP DAAC".into(),
            start_date: "2015-11-28".into(),
            end_date: "present".into(),
            native_scale_m: 30.0,
            cloud_property: Some("CLOUD_COVERAGE".into()),
            default_bands: vec!["B4".into(), "B3".into(), "B2".into()],
            vis_min: 0.0,
            vis_max: 0.3,
            tags: "hls hlss30 sentinel-2 harmonized surface reflectance optical 跨传感器一致化 光学 地表反射率".into(),
            asset_url: "https://developers.google.com/earth-engine/datasets/catalog/NASA_HLS_HLSS30_v002".into(),
        },
    ]
}

/// Known explicit spatial bounds `[west, south, east, north]` for regionally restricted GEE datasets.
pub fn known_asset_spatial_bounds(asset_id: &str) -> Option<[f64; 4]> {
    let clean = asset_id.trim();
    if clean.eq_ignore_ascii_case("projects/sat-io/open-datasets/CLCD") {
        // Wuhan University China Land Cover Dataset covers mainland China & immediate margins only.
        return Some([73.0, 18.0, 136.0, 54.0]);
    }
    None
}

pub fn lookup_dataset_profile(asset_id: &str) -> Option<GeeDatasetProfile> {
    let trimmed = asset_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    let datasets = builtin_curated_datasets();
    if let Some(exact) = datasets.iter().find(|d| d.id.eq_ignore_ascii_case(trimmed)) {
        return Some(exact.clone());
    }
    // Single-scene child asset ID inside a curated ImageCollection (e.g. COPERNICUS/S2_SR_HARMONIZED/<granule>)
    let lower_trimmed = trimmed.to_ascii_lowercase();
    if let Some(parent) = datasets.into_iter().find(|d| {
        if d.asset_type != "image_collection" {
            return false;
        }
        let prefix = format!("{}/", d.id.to_ascii_lowercase());
        lower_trimmed.starts_with(&prefix)
    }) {
        let mut cloned = parent;
        cloned.id = trimmed.to_owned();
        cloned.asset_type = "image".to_owned();
        return Some(cloned);
    }
    None
}

fn expand_bilingual_query_terms(raw_query: &str) -> Vec<String> {
    let lower = raw_query.trim().to_lowercase();
    if lower.is_empty() {
        return Vec::new();
    }
    let mut terms: Vec<String> = lower
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '|')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if !terms.contains(&lower) {
        terms.push(lower.clone());
    }

    const SYNONYMS: &[(&[&str], &[&str])] = &[
        (
            &["哨兵2", "哨兵二", "光学"],
            &["sentinel-2", "s2_sr_harmonized", "optical"],
        ),
        (
            &["哨兵1", "哨兵一", "雷达", "合成孔径"],
            &["sentinel-1", "s1_grd", "sar", "radar", "palsar"],
        ),
        (
            &["陆地卫星", "landsat"],
            &["landsat", "lc09", "lc08", "lt05"],
        ),
        (
            &["夜间灯光", "夜光", "灯光"],
            &["viirs", "dnb", "nighttime", "lights", "dmsp"],
        ),
        (
            &["地表温度", "热红外", "热岛", "温度"],
            &["lst", "mod11a1", "thermal", "temperature", "lc09"],
        ),
        (&["土壤湿度", "墒情"], &["smap", "soil", "moisture", "era5"]),
        (
            &["降水", "降雨", "暴雨", "台风"],
            &["precipitation", "chirps", "gpm", "imerg", "rain"],
        ),
        (
            &["气象", "再分析", "风速", "气温"],
            &["era5", "reanalysis", "climate", "temperature"],
        ),
        (
            &["土地覆盖", "土地利用", "地物", "建成区"],
            &["landcover", "lulc", "worldcover", "dynamicworld", "clcd"],
        ),
        (
            &["水体", "洪水", "湖泊", "河流", "淹没"],
            &["water", "gsw", "flood", "surface", "jrc"],
        ),
        (
            &["植被", "绿度", "生态", "ndvi"],
            &["ndvi", "evi", "vegetation", "mod13q1", "s2_sr"],
        ),
        (
            &["高程", "地形", "dem", "裸地", "15米"],
            &[
                "dem",
                "elevation",
                "glo30",
                "nasadem",
                "srtm",
                "fabdem",
                "gem-15",
                "dtm",
                "dsm",
            ],
        ),
        (
            &["树高", "冠层", "激光雷达", "生物量"],
            &["gedi", "canopy", "biomass", "lidar", "height"],
        ),
        (
            &["人口", "人口密度"],
            &["worldpop", "population", "ghsl", "gpw"],
        ),
        (&["建筑高度", "建筑"], &["building", "ghsl", "urban"]),
        (
            &["火灾", "火点", "火烧", "山火"],
            &["fire", "firms", "burned", "wildfire"],
        ),
        (
            &["大气", "二氧化氮", "空气质量", "污染", "臭氧", "甲烷"],
            &["s5p", "no2", "atmosphere", "aerosol", "ch4", "o3"],
        ),
    ];

    for (cn_keys, en_expansions) in SYNONYMS {
        if cn_keys.iter().any(|k| lower.contains(k)) {
            for exp in *en_expansions {
                let s = (*exp).to_owned();
                if !terms.contains(&s) {
                    terms.push(s);
                }
            }
        }
    }
    terms
}

fn cache_is_fresh(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.len() < 128 {
        return false;
    }
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    age < CACHE_TTL_SECS
}

fn ensure_cached_catalog_json(
    client: &reqwest::blocking::Client,
    cache_file: &Path,
    url: &str,
    refresh_online: bool,
) -> Option<Vec<Value>> {
    if !refresh_online && cache_file.is_file() {
        if let Ok(raw) = fs::read_to_string(cache_file) {
            if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&raw) {
                if !arr.is_empty() {
                    return Some(arr);
                }
            }
        }
    }
    if cache_is_fresh(cache_file) && !refresh_online {
        if let Ok(raw) = fs::read_to_string(cache_file) {
            if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&raw) {
                return Some(arr);
            }
        }
    }

    if let Ok(resp) = client.get(url).send() {
        if resp.status().is_success() {
            if let Ok(text) = resp.text() {
                if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&text) {
                    if !arr.is_empty() {
                        if let Some(parent) = cache_file.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(cache_file, &text);
                        return Some(arr);
                    }
                }
            }
        }
    }

    if cache_file.is_file() {
        if let Ok(raw) = fs::read_to_string(cache_file) {
            if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&raw) {
                return Some(arr);
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct CatalogSearchOptions {
    pub include_community: bool,
    pub limit: usize,
    pub force_refresh: bool,
    pub source_filter: Option<String>,
    pub provider_filter: Option<String>,
    pub kind_filter: Option<String>,
    pub max_resolution_m: Option<f64>,
    pub include_deprecated: bool,
}

impl Default for CatalogSearchOptions {
    fn default() -> Self {
        Self {
            include_community: true,
            limit: 24,
            force_refresh: false,
            source_filter: None,
            provider_filter: None,
            kind_filter: None,
            max_resolution_m: None,
            include_deprecated: false,
        }
    }
}

#[derive(Clone, Debug)]
struct ConceptSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    preferred_ids: &'static [&'static str],
}

const CONCEPTS: &[ConceptSpec] = &[
    ConceptSpec {
        name: "terrain-dem",
        aliases: &[
            "dem",
            "digital elevation",
            "elevation",
            "terrain",
            "slope",
            "hillshade",
            "高程",
            "数字高程",
            "地形",
            "坡度",
            "山地",
        ],
        preferred_ids: &[
            "COPERNICUS/DEM/GLO30_2024_1",
            "COPERNICUS/DEM/GLO30",
            "USGS/SRTMGL1_003",
            "NASA/NASADEM_HGT/001",
            "projects/sat-io/open-datasets/FABDEM",
        ],
    },
    ConceptSpec {
        name: "soil-moisture",
        aliases: &[
            "soil moisture",
            "smap",
            "root zone moisture",
            "surface moisture",
            "土壤湿度",
            "土壤水分",
            "墒情",
        ],
        preferred_ids: &[
            "NASA/SMAP/SPL4SMGP/008",
            "NASA/SMAP/SPL4SMGP/007",
            "ECMWF/ERA5_LAND/DAILY_AGGR",
        ],
    },
    ConceptSpec {
        name: "precipitation",
        aliases: &[
            "precipitation",
            "rainfall",
            "rain",
            "gpm",
            "imerg",
            "chirps",
            "降水",
            "降雨",
            "暴雨",
            "雨量",
            "台风",
        ],
        preferred_ids: &[
            "NASA/GPM_L3/IMERG_V07",
            "UCSB-CHG/CHIRPS/DAILY",
            "ECMWF/ERA5_LAND/DAILY_AGGR",
        ],
    },
    ConceptSpec {
        name: "flood-water",
        aliases: &[
            "flood",
            "inundation",
            "surface water",
            "water extent",
            "water occurrence",
            "lake",
            "river",
            "洪水",
            "洪涝",
            "淹没",
            "积水",
            "内涝",
            "水体",
            "湖泊",
            "河流",
        ],
        preferred_ids: &[
            "COPERNICUS/S1_GRD",
            "JRC/GSW1_4/GlobalSurfaceWater",
            "JRC/GSW1_4/MonthlyHistory",
        ],
    },
    ConceptSpec {
        name: "vegetation",
        aliases: &[
            "vegetation",
            "ndvi",
            "evi",
            "greenness",
            "phenology",
            "crop",
            "植被",
            "长势",
            "物候",
            "作物",
            "农田",
            "农情",
            "绿度",
            "生态",
        ],
        preferred_ids: &[
            "COPERNICUS/S2_SR_HARMONIZED",
            "MODIS/061/MOD13Q1",
            "LANDSAT/LC09/C02/T1_L2",
            "NASA/HLS/HLSS30/v002",
        ],
    },
    ConceptSpec {
        name: "optical",
        aliases: &[
            "optical",
            "sentinel-2",
            "sentinel 2",
            "landsat",
            "hls",
            "rgb",
            "surface reflectance",
            "光学",
            "真彩色",
            "地表反射率",
            "哨兵2",
            "哨兵二",
            "陆地卫星",
        ],
        preferred_ids: &[
            "COPERNICUS/S2_SR_HARMONIZED",
            "LANDSAT/LC09/C02/T1_L2",
            "LANDSAT/LC08/C02/T1_L2",
            "NASA/HLS/HLSS30/v002",
            "NASA/HLS/HLSL30/v002",
        ],
    },
    ConceptSpec {
        name: "radar",
        aliases: &[
            "radar",
            "sar",
            "sentinel-1",
            "sentinel 1",
            "backscatter",
            "palsar",
            "雷达",
            "哨兵1",
            "哨兵一",
            "后向散射",
            "合成孔径",
        ],
        preferred_ids: &["COPERNICUS/S1_GRD", "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH"],
    },
    ConceptSpec {
        name: "land-cover",
        aliases: &[
            "land cover",
            "landcover",
            "lulc",
            "classification",
            "dynamic world",
            "worldcover",
            "clcd",
            "土地覆盖",
            "土地利用",
            "地类",
            "地物",
            "分类",
            "建成区",
        ],
        preferred_ids: &[
            "GOOGLE/DYNAMICWORLD/V1",
            "ESA/WorldCover/v200",
            "projects/sat-io/open-datasets/CLCD",
        ],
    },
    ConceptSpec {
        name: "surface-temperature",
        aliases: &[
            "land surface temperature",
            "surface temperature",
            "lst",
            "thermal",
            "heat island",
            "地表温度",
            "热岛",
            "热红外",
            "温度",
        ],
        preferred_ids: &[
            "MODIS/061/MOD11A1",
            "LANDSAT/LC09/C02/T1_L2",
            "LANDSAT/LC08/C02/T1_L2",
        ],
    },
    ConceptSpec {
        name: "population-exposure",
        aliases: &[
            "population",
            "exposure",
            "vulnerability",
            "worldpop",
            "ghsl",
            "人口",
            "暴露",
            "受灾人口",
            "人口密度",
            "人口分布",
        ],
        preferred_ids: &["WorldPop/GP/100m/pop", "JRC/GHSL/P2023A/GHS_POP"],
    },
    ConceptSpec {
        name: "buildings",
        aliases: &[
            "building",
            "buildings",
            "footprint",
            "open buildings",
            "建筑",
            "建筑物",
            "房屋",
            "建筑高度",
        ],
        preferred_ids: &[
            "GOOGLE/Research/open-buildings/v3/polygons",
            "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M",
        ],
    },
    ConceptSpec {
        name: "night-lights",
        aliases: &[
            "night light",
            "nighttime light",
            "viirs",
            "dnb",
            "夜间灯光",
            "夜光",
            "城市扩张",
        ],
        preferred_ids: &["NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG"],
    },
    ConceptSpec {
        name: "fire",
        aliases: &[
            "fire",
            "wildfire",
            "burned area",
            "active fire",
            "firms",
            "野火",
            "火灾",
            "火烧",
            "火烧迹地",
            "火点",
            "山火",
        ],
        preferred_ids: &["MODIS/061/MCD64A1", "FIRMS"],
    },
    ConceptSpec {
        name: "forest",
        aliases: &[
            "forest",
            "tree cover",
            "canopy",
            "gedi",
            "deforestation",
            "森林",
            "林地",
            "树冠",
            "树高",
            "毁林",
        ],
        preferred_ids: &[
            "UMD/hansen/global_forest_change_2023_v1_11",
            "LARSE/GEDI/GEDI02_A_002_MONTHLY",
        ],
    },
    ConceptSpec {
        name: "climate",
        aliases: &[
            "climate",
            "weather",
            "temperature",
            "wind",
            "era5",
            "气候",
            "气象",
            "再分析",
            "气温",
            "风速",
        ],
        preferred_ids: &["ECMWF/ERA5_LAND/DAILY_AGGR", "ECMWF/ERA5_LAND/HOURLY"],
    },
    ConceptSpec {
        name: "air-quality",
        aliases: &[
            "air quality",
            "pollution",
            "no2",
            "s5p",
            "空气质量",
            "污染",
            "二氧化氮",
            "大气",
        ],
        preferred_ids: &["COPERNICUS/S5P/OFFL/L3_NO2"],
    },
];

pub(crate) fn dataset_deprecation_info(id: &str) -> (bool, Option<&'static str>) {
    let trimmed = id.trim();
    match trimmed {
        "COPERNICUS/S2_SR" => (true, Some("COPERNICUS/S2_SR_HARMONIZED")),
        "COPERNICUS/S2" => (true, Some("COPERNICUS/S2_HARMONIZED")),
        "LANDSAT/LC08/C01/T1_SR" => (true, Some("LANDSAT/LC08/C02/T1_L2")),
        "LANDSAT/LC08/C01/T1_TOA" => (true, Some("LANDSAT/LC08/C02/T1_TOA")),
        "LANDSAT/LE07/C01/T1_SR" => (true, Some("LANDSAT/LE07/C02/T1_L2")),
        "LANDSAT/LT05/C01/T1_SR" => (true, Some("LANDSAT/LT05/C02/T1_L2")),
        "MODIS/006/MOD13Q1" => (true, Some("MODIS/061/MOD13Q1")),
        "MODIS/006/MOD11A1" => (true, Some("MODIS/061/MOD11A1")),
        "MODIS/006/MCD64A1" => (true, Some("MODIS/061/MCD64A1")),
        "JRC/GSW1_3/GlobalSurfaceWater" => (true, Some("JRC/GSW1_4/GlobalSurfaceWater")),
        "JRC/GSW1_3/MonthlyHistory" => (true, Some("JRC/GSW1_4/MonthlyHistory")),
        "ESA/WorldCover/v100" => (true, Some("ESA/WorldCover/v200")),
        "NASA/GPM_L3/IMERG_V06" => (true, Some("NASA/GPM_L3/IMERG_V07")),
        _ => (false, None),
    }
}

pub(crate) fn curated_science_guidance(id: &str) -> (&'static str, &'static str) {
    let u = id.trim().to_ascii_uppercase();
    if u.starts_with("COPERNICUS/S2_SR") {
        (
            "表面反射率比例因子 0.0001（原始 DN 0..10000；归一化指数可直接使用，物理量需乘 0.0001）",
            "先按 CLOUDY_PIXEL_PERCENTAGE 滤景，再使用 Cloud Score+ 或 SCL 做像元级去云（避免跨 2022–2024 仅依赖 QA60）",
        )
    } else if u.contains("/C02/T1_L2") {
        (
            "光学反射率：SR_B* × 0.0000275 - 0.2；热红外温度：ST_B* × 0.00341802 + 149.0 K",
            "必须对 QA_PIXEL 位 0–5（填充·膨胀云·卷云·云·云影·雪）及 QA_RADSAT 饱和像元掩膜",
        )
    } else if u.starts_with("NASA/HLS/") {
        (
            "30 米跨传感器 NBAR 反射率，常见比例因子 0.0001（单位：反射率）",
            "使用 Fmask 位掩膜排除云、云影、雪冰和邻近云像元，并记录 L30/S30 传感器来源",
        )
    } else if u == "COPERNICUS/S1_GRD" {
        (
            "VV/VH 默认为对数 dB 尺度；做比值或面积加权平均前需先转为线性功率",
            "建议按轨道升降轨（ASCENDING/DESCENDING）与相对轨道号分组对比，减少观测几何差异",
        )
    } else if u.starts_with("MODIS/061/MOD13Q1") {
        (
            "NDVI/EVI 比例因子 0.0001（物理有效范围 -0.2..1.0）",
            "时序分析建议结合 DetailedQA 或 SummaryQA 过滤低质量合成像元",
        )
    } else if u.starts_with("MODIS/061/MOD11A1") {
        (
            "LST_Day_1km / LST_Night_1km 比例因子 0.02（单位：开尔文 K，减去 273.15 转为摄氏度）",
            "结合 QC_Day / QC_Night 质量掩膜去除云污染像元",
        )
    } else if u.starts_with("ESA/WORLDCOVER")
        || u.starts_with("GOOGLE/DYNAMICWORLD")
        || u.contains("/CLCD")
    {
        (
            "离散地物分类编码（无量纲），严禁直接求空间或时间算术平均",
            "分类图应使用众数 mode()、分组统计或乘以 pixelArea() 统计类别面积；Dynamic World 宜检查类别概率波段",
        )
    } else if u.contains("DEM")
        || u.contains("SRTM")
        || u.contains("FABDEM")
        || u.contains("GEM-15")
    {
        (
            "高程单位为米（m，多为正高或 EGM96/EGM2008 基准）",
            "静态地形基准资产通常无需日期与云量过滤，坡度/水文分析前须注意投影与尺度",
        )
    } else {
        (
            "请以数据集官方文档或 STAC 波段说明中的 scale/offset 与单位为准",
            "分析或导出前请核实有效值范围、NoData 与适用质量掩膜",
        )
    }
}

fn verification_rule_for_source(source: &str, deprecated: bool) -> &'static str {
    if deprecated {
        return "该数据集已标记退役或被新版替代，建议改用推荐替代版本后再开展分析与导出。";
    }
    if source == "community" {
        "社区托管目录仅作发现提示：分析前须核验资产实时存在性、许可协议、波段定标与更新状态。"
    } else {
        "官方目录记录：编码前请核对资产 ID、波段名、定标系数、QA 掩膜与研究区时空覆盖。"
    }
}

fn detect_query_concepts(raw_query: &str) -> Vec<&'static str> {
    let lower = raw_query.to_ascii_lowercase();
    let mut out = Vec::new();
    for spec in CONCEPTS {
        if spec.aliases.iter().any(|alias| lower.contains(alias)) {
            out.push(spec.name);
        }
    }
    out
}

fn entry_is_deprecated(id: &str, entry: &Value) -> (bool, Option<String>) {
    let (gov_dep, gov_repl) = dataset_deprecation_info(id);
    if gov_dep {
        return (true, gov_repl.map(str::to_owned));
    }
    let dep_flag = entry["deprecated"].as_bool() == Some(true)
        || entry["is_deprecated"].as_bool() == Some(true);
    let title_l = entry["title"].as_str().unwrap_or("").to_ascii_lowercase();
    let tags_l = entry["tags"].as_str().unwrap_or("").to_ascii_lowercase();
    let dep_text = title_l.contains("deprecated") || tags_l.contains("deprecated");
    let repl = entry["successor_id"]
        .as_str()
        .or_else(|| entry["replacement_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    (dep_flag || dep_text || repl.is_some(), repl)
}

fn parse_optional_scale_m(entry: &Value) -> Option<f64> {
    entry["native_scale_m"]
        .as_f64()
        .or_else(|| entry["resolution_m"].as_f64())
        .or_else(|| entry["pixel_size"].as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn matches_catalog_filters(
    source: &str,
    curated: bool,
    provider: &str,
    kind: &str,
    native_scale_m: Option<f64>,
    deprecated: bool,
    options: &CatalogSearchOptions,
) -> bool {
    if !options.include_deprecated && deprecated {
        return false;
    }
    if let Some(ref src_f) = options.source_filter {
        let sf = src_f.trim().to_ascii_lowercase();
        if !sf.is_empty() {
            if sf == "curated" {
                if !curated {
                    return false;
                }
            } else if !source.eq_ignore_ascii_case(&sf) {
                return false;
            }
        }
    }
    if let Some(ref prov_f) = options.provider_filter {
        let pf = prov_f.trim().to_ascii_lowercase();
        if !pf.is_empty() && !provider.to_ascii_lowercase().contains(&pf) {
            return false;
        }
    }
    if let Some(ref kind_f) = options.kind_filter {
        let kf = kind_f.trim().to_ascii_lowercase();
        if !kf.is_empty() && !kind.to_ascii_lowercase().contains(&kf) {
            return false;
        }
    }
    if let Some(max_res) = options.max_resolution_m {
        match native_scale_m {
            Some(scale) if scale.is_finite() && scale <= max_res => {}
            _ => return false,
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn score_entry_with_reasons(
    id: &str,
    title: &str,
    title_zh: &str,
    tags: &str,
    provider: &str,
    raw_query_lower: &str,
    terms: &[String],
    concepts: &[&str],
    curated_boost: i64,
    deprecated: bool,
) -> (i64, Vec<String>) {
    if raw_query_lower.is_empty() {
        let mut reasons = vec!["默认精选目录".to_owned()];
        if deprecated {
            reasons.push("已标记退役".to_owned());
        }
        return (10 + curated_boost, reasons);
    }

    let id_l = id.to_lowercase();
    let title_l = title.to_lowercase();
    let zh_l = title_zh.to_lowercase();
    let tags_l = tags.to_lowercase();
    let prov_l = provider.to_lowercase();

    let mut score = 0i64;
    let mut reasons: Vec<String> = Vec::new();

    if id_l == raw_query_lower {
        score += 500;
        reasons.push("精确匹配资产 ID".to_owned());
    } else if id_l.contains(raw_query_lower) {
        score += 220;
        reasons.push("资产 ID 匹配".to_owned());
    }
    if !zh_l.is_empty() && zh_l.contains(raw_query_lower) {
        score += 200;
        reasons.push("中文标题直达".to_owned());
    }
    if title_l.contains(raw_query_lower) {
        score += 150;
        reasons.push("英文标题直达".to_owned());
    }

    let mut id_hits = 0usize;
    let mut title_hits = 0usize;
    let mut tag_hits = 0usize;
    for term in terms {
        if id_l.contains(term) {
            score += 75;
            id_hits += 1;
        }
        if !zh_l.is_empty() && zh_l.contains(term) {
            score += 70;
            title_hits += 1;
        }
        if title_l.contains(term) {
            score += 55;
            title_hits += 1;
        }
        if tags_l.contains(term) {
            score += 35;
            tag_hits += 1;
        }
        if prov_l.contains(term) {
            score += 15;
        }
    }
    if id_hits > 0 && reasons.is_empty() {
        reasons.push(format!("ID 关键词命中 {id_hits} 项"));
    }
    if title_hits > 0 {
        reasons.push(format!("标题命中 {title_hits} 项"));
    }
    if tag_hits > 0 {
        reasons.push(format!("主题标签命中 {tag_hits} 项"));
    }

    for &cname in concepts {
        if let Some(spec) = CONCEPTS.iter().find(|c| c.name == cname) {
            if spec
                .preferred_ids
                .iter()
                .any(|p| p.eq_ignore_ascii_case(id))
            {
                score += 65;
                reasons.push(format!("主题 {cname} 首选现行产品"));
            } else if spec
                .aliases
                .iter()
                .any(|a| tags_l.contains(a) || title_l.contains(a) || zh_l.contains(a))
            {
                score += 20;
                reasons.push(format!("符合 {cname} 主题语义"));
            }
        }
    }

    if score <= 0 {
        return (0, Vec::new());
    }

    score += curated_boost;
    if deprecated {
        score = (score - 120).max(1);
        reasons.push("已标记退役 · 建议改用替代版本".to_owned());
    }
    if reasons.len() > 6 {
        reasons.truncate(6);
    }
    (score, reasons)
}

fn confidence_label(score: i64, is_empty_query: bool) -> &'static str {
    if is_empty_query || score >= 140 {
        "high"
    } else if score >= 60 {
        "medium"
    } else {
        "low"
    }
}

pub fn search_gee_catalog(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    query: &str,
    include_community: bool,
    limit: usize,
    force_refresh: bool,
) -> Value {
    let options = CatalogSearchOptions {
        include_community,
        limit,
        force_refresh,
        ..CatalogSearchOptions::default()
    };
    search_gee_catalog_with_options(app_data_dir, client, query, &options)
}

pub fn search_gee_catalog_with_options(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    query: &str,
    options: &CatalogSearchOptions,
) -> Value {
    if let Some(max_res) = options.max_resolution_m {
        if !max_res.is_finite() || max_res <= 0.0 {
            return json!({
                "ok": false,
                "error": "INVALID_MAX_RESOLUTION_M",
                "message_zh": "空间分辨率上限 max_resolution_m 必须为大于 0 的有限数值（单位：米）。",
                "dataset": "gee_catalog_search",
                "query": query.trim(),
                "count": 0,
                "total_matches": 0,
                "items": [],
                "needs_clarification": true,
            });
        }
    }

    let max_items = options.limit.clamp(1, 100);
    let raw_q = query.trim();
    let raw_q_lower = raw_q.to_lowercase();
    let terms = expand_bilingual_query_terms(raw_q);
    let concepts = detect_query_concepts(raw_q);

    let curated = builtin_curated_datasets();
    let mut seen_ids = HashSet::new();
    let mut scored: Vec<(i64, Value)> = Vec::new();

    for item in &curated {
        if !options.include_community && item.source == "community" {
            continue;
        }
        seen_ids.insert(item.id.to_lowercase());
        let (deprecated, replacement_id) = dataset_deprecation_info(&item.id);
        if !matches_catalog_filters(
            &item.source,
            true,
            &item.provider,
            &item.asset_type,
            Some(item.native_scale_m),
            deprecated,
            options,
        ) {
            continue;
        }
        let (sc, reasons) = score_entry_with_reasons(
            &item.id,
            &item.title,
            &item.title_zh,
            &item.tags,
            &item.provider,
            &raw_q_lower,
            &terms,
            &concepts,
            40,
            deprecated,
        );
        if sc > 0 {
            let conf = confidence_label(sc, raw_q_lower.is_empty());
            let mut row = json!({
                "id": item.id,
                "title": item.title,
                "title_zh": item.title_zh,
                "source": item.source,
                "type": item.asset_type,
                "provider": item.provider,
                "start_date": item.start_date,
                "end_date": item.end_date,
                "native_scale_m": item.native_scale_m,
                "cloud_property": item.cloud_property,
                "default_bands": item.default_bands,
                "vis_min": item.vis_min,
                "vis_max": item.vis_max,
                "tags": item.tags,
                "asset_url": item.asset_url,
                "curated": true,
                "deprecated": deprecated,
                "score": sc,
                "confidence": conf,
                "match_reasons": reasons,
                "verification_rule": verification_rule_for_source(&item.source, deprecated),
            });
            if let Some(repl) = replacement_id {
                row["replacement_id"] = json!(repl);
            }
            scored.push((sc, row));
        }
    }

    let cache_dir = app_data_dir.join("gee_catalog_cache");
    let official_cache = cache_dir.join("official_catalog.json");
    let community_cache = cache_dir.join("community_catalog.json");

    let mut total_official_indexed = curated.iter().filter(|d| d.source == "official").count();
    let mut total_community_indexed = curated.iter().filter(|d| d.source == "community").count();

    let official_entries = if let Some(http) = client {
        ensure_cached_catalog_json(
            http,
            &official_cache,
            OFFICIAL_CATALOG_URL,
            options.force_refresh,
        )
    } else if official_cache.is_file() {
        fs::read_to_string(&official_cache)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
    } else {
        None
    };

    if let Some(entries) = official_entries {
        total_official_indexed = entries.len();
        for entry in entries {
            let id = entry["id"].as_str().unwrap_or("").trim();
            if id.is_empty() || !seen_ids.insert(id.to_lowercase()) {
                continue;
            }
            let title = entry["title"].as_str().unwrap_or(id);
            let tags = entry["tags"].as_str().unwrap_or("");
            let provider = entry["provider"].as_str().unwrap_or("");
            let kind = entry["type"].as_str().unwrap_or("image_collection");
            let native_scale_m = parse_optional_scale_m(&entry);
            let (deprecated, replacement_id) = entry_is_deprecated(id, &entry);
            if !matches_catalog_filters(
                "official",
                false,
                provider,
                kind,
                native_scale_m,
                deprecated,
                options,
            ) {
                continue;
            }
            let (sc, reasons) = score_entry_with_reasons(
                id,
                title,
                "",
                tags,
                provider,
                &raw_q_lower,
                &terms,
                &concepts,
                0,
                deprecated,
            );
            if sc > 0 {
                let conf = confidence_label(sc, raw_q_lower.is_empty());
                let mut row = json!({
                    "id": id,
                    "title": title,
                    "title_zh": "",
                    "source": "official",
                    "type": kind,
                    "provider": provider,
                    "start_date": entry["start_date"].as_str().unwrap_or(""),
                    "end_date": entry["end_date"].as_str().unwrap_or(""),
                    "tags": tags,
                    "asset_url": entry["asset_url"].as_str().unwrap_or(""),
                    "curated": false,
                    "deprecated": deprecated,
                    "score": sc,
                    "confidence": conf,
                    "match_reasons": reasons,
                    "verification_rule": verification_rule_for_source("official", deprecated),
                });
                if let Some(scale) = native_scale_m {
                    row["native_scale_m"] = json!(scale);
                }
                if let Some(repl) = replacement_id {
                    row["replacement_id"] = json!(repl);
                }
                scored.push((sc, row));
            }
        }
    }

    if options.include_community {
        let community_entries = if let Some(http) = client {
            if community_cache.is_file()
                || options.force_refresh
                || (!raw_q.is_empty() && scored.len() < 8)
            {
                ensure_cached_catalog_json(
                    http,
                    &community_cache,
                    COMMUNITY_CATALOG_URL,
                    options.force_refresh,
                )
            } else {
                None
            }
        } else if community_cache.is_file() {
            fs::read_to_string(&community_cache)
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        } else {
            None
        };

        if let Some(entries) = community_entries {
            total_community_indexed = entries.len();
            for entry in entries {
                let id = entry["id"].as_str().unwrap_or("").trim();
                if id.is_empty() || !seen_ids.insert(id.to_lowercase()) {
                    continue;
                }
                let title = entry["title"].as_str().unwrap_or(id);
                let tags = entry["tags"].as_str().unwrap_or("");
                let provider = entry["provider"].as_str().unwrap_or("");
                let kind = entry["type"].as_str().unwrap_or("image_collection");
                let native_scale_m = parse_optional_scale_m(&entry);
                let (deprecated, replacement_id) = entry_is_deprecated(id, &entry);
                if !matches_catalog_filters(
                    "community",
                    false,
                    provider,
                    kind,
                    native_scale_m,
                    deprecated,
                    options,
                ) {
                    continue;
                }
                let (sc, reasons) = score_entry_with_reasons(
                    id,
                    title,
                    "",
                    tags,
                    provider,
                    &raw_q_lower,
                    &terms,
                    &concepts,
                    -5,
                    deprecated,
                );
                if sc > 0 {
                    let conf = confidence_label(sc, raw_q_lower.is_empty());
                    let mut row = json!({
                        "id": id,
                        "title": title,
                        "title_zh": "",
                        "source": "community",
                        "type": kind,
                        "provider": provider,
                        "start_date": "",
                        "end_date": "",
                        "tags": tags,
                        "asset_url": entry["docs"].as_str().unwrap_or(""),
                        "license": entry["license"].as_str().unwrap_or(""),
                        "curated": false,
                        "deprecated": deprecated,
                        "score": sc,
                        "confidence": conf,
                        "match_reasons": reasons,
                        "verification_rule": verification_rule_for_source("community", deprecated),
                    });
                    if let Some(scale) = native_scale_m {
                        row["native_scale_m"] = json!(scale);
                    }
                    if let Some(repl) = replacement_id {
                        row["replacement_id"] = json!(repl);
                    }
                    scored.push((sc, row));
                }
            }
        }
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| {
                let ad = a.1["deprecated"].as_bool().unwrap_or(false);
                let bd = b.1["deprecated"].as_bool().unwrap_or(false);
                ad.cmp(&bd)
            })
            .then_with(|| {
                let ao = a.1["source"].as_str() != Some("official");
                let bo = b.1["source"].as_str() != Some("official");
                ao.cmp(&bo)
            })
    });
    let total_matches = scored.len();
    let results: Vec<Value> = scored.into_iter().take(max_items).map(|(_, v)| v).collect();

    let needs_clarification = if raw_q_lower.is_empty() {
        false
    } else {
        results.is_empty() || results[0]["confidence"].as_str() == Some("low")
    };
    let clarification_zh = if needs_clarification {
        Some(
            "未找到高置信度匹配数据集；请补充目标物理量、传感器类型（光学或雷达）、时间范围或空间分辨率要求。"
                .to_owned(),
        )
    } else {
        None
    };

    json!({
        "ok": true,
        "dataset": "gee_catalog_search",
        "query": raw_q,
        "expanded_terms": terms,
        "detected_concepts": concepts,
        "include_community": options.include_community,
        "include_deprecated": options.include_deprecated,
        "total_official_indexed": total_official_indexed,
        "total_community_indexed": total_community_indexed,
        "total_matches": total_matches,
        "count": results.len(),
        "items": results,
        "needs_clarification": needs_clarification,
        "clarification_zh": clarification_zh,
        "updated_at": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

struct TaskBundleRecipe {
    id: &'static str,
    title_zh: &'static str,
    markers: &'static [&'static str],
    roles: &'static [(&'static str, &'static str, &'static str)],
}

const TASK_BUNDLE_RECIPES: &[TaskBundleRecipe] = &[
    TaskBundleRecipe {
        id: "flood-risk",
        title_zh: "洪涝灾害与淹没风险评估配数方案",
        markers: &[
            "flood",
            "inundation",
            "洪涝",
            "洪水",
            "淹没",
            "内涝",
            "水体风险",
        ],
        roles: &[
            (
                "hazard_event",
                "灾情水体观测",
                "sentinel-1 sar radar flood water 洪水 雷达",
            ),
            (
                "water_baseline",
                "长期水体基线",
                "global surface water jrc gsw occurrence 水体",
            ),
            (
                "terrain",
                "地形与汇水背景",
                "dem elevation bare-earth fabdem glo30 高程 地形",
            ),
            (
                "rainfall",
                "强降雨驱动",
                "gpm imerg chirps precipitation rainfall 降雨 暴雨",
            ),
            (
                "exposure",
                "人口与建筑暴露",
                "worldpop ghsl open buildings population 人口 建筑",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "vegetation-monitoring",
        title_zh: "植被长势与物候时序监测配数方案",
        markers: &[
            "vegetation",
            "ndvi",
            "phenology",
            "crop",
            "植被",
            "长势",
            "物候",
            "农情",
            "绿度",
        ],
        roles: &[
            (
                "high_res_optical",
                "高分光学反射率",
                "sentinel-2 landsat hls optical 光学",
            ),
            (
                "time_series_vi",
                "长时序植被指数",
                "mod13q1 modis ndvi evi vegetation 植被指数",
            ),
            (
                "climate_context",
                "气候水分背景",
                "era5 climate precipitation temperature 气象",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "soil-moisture",
        title_zh: "土壤墒情与农业干旱监测配数方案",
        markers: &[
            "soil moisture",
            "drought",
            "土壤湿度",
            "土壤水分",
            "墒情",
            "干旱",
        ],
        roles: &[
            (
                "primary_observation",
                "表层与根区土壤湿度",
                "smap soil moisture 土壤湿度",
            ),
            (
                "meteorological_context",
                "气象水热强迫",
                "era5 precipitation temperature climate 降水 气温",
            ),
            (
                "vegetation_response",
                "植被胁迫响应",
                "mod13q1 ndvi vegetation 植被",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "land-cover",
        title_zh: "土地覆盖分类与动态变化配数方案",
        markers: &[
            "land cover",
            "landcover",
            "lulc",
            "classification",
            "土地覆盖",
            "土地利用",
            "地物分类",
            "地类",
        ],
        roles: &[
            (
                "dynamic_product",
                "准实时动态地类",
                "dynamic world land cover 土地覆盖",
            ),
            (
                "static_baseline",
                "基准地类参考",
                "worldcover clcd land cover baseline 基准",
            ),
            (
                "source_imagery",
                "源多光谱影像",
                "sentinel-2 optical surface reflectance 光学",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "urban-heat",
        title_zh: "城市热岛与地表温度分析配数方案",
        markers: &[
            "heat island",
            "lst",
            "surface temperature",
            "thermal",
            "热岛",
            "地表温度",
            "热红外",
        ],
        roles: &[
            (
                "thermal_lst",
                "热红外地表温度",
                "mod11a1 landsat lst thermal 地表温度",
            ),
            (
                "optical_context",
                "不透水面与植被覆盖",
                "sentinel-2 landsat optical 光学",
            ),
            (
                "urban_exposure",
                "人口与建筑密度背景",
                "ghsl worldpop open buildings 人口 建筑",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "wildfire",
        title_zh: "森林火灾与火烧烈度评估配数方案",
        markers: &[
            "fire", "wildfire", "burn", "burned", "火灾", "野火", "火烧", "山火", "火点",
        ],
        roles: &[
            ("burned_area", "过火区月度产品", "mcd64a1 burned area 火烧"),
            (
                "active_fire",
                "近实时热异常火点",
                "firms active fire thermal anomaly 火点",
            ),
            (
                "pre_post_optical",
                "灾前灾后火烧指数源影像",
                "sentinel-2 landsat nbr optical 光学",
            ),
        ],
    },
    TaskBundleRecipe {
        id: "geohazard-terrain",
        title_zh: "地震滑坡与地质灾害环境配数方案",
        markers: &[
            "earthquake",
            "landslide",
            "seismic",
            "deformation",
            "地震",
            "滑坡",
            "地质灾害",
            "形变",
        ],
        roles: &[
            (
                "sar_observation",
                "全天候雷达形变与强度",
                "sentinel-1 palsar sar radar 雷达",
            ),
            (
                "terrain_dem",
                "高程与坡度地形背景",
                "dem elevation glo30 fabdem 高程 地形",
            ),
            (
                "optical_change",
                "高分光学裂隙与滑坡体识别",
                "sentinel-2 landsat optical 光学",
            ),
        ],
    },
];

pub fn recommend_gee_dataset_bundle(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    task: &str,
    limit_per_role: usize,
    include_community: bool,
) -> Value {
    let trimmed = task.trim();
    if trimmed.is_empty() {
        return json!({
            "ok": true,
            "dataset": "gee_recommend_datasets",
            "task": "",
            "recipe": null,
            "roles": {},
            "missing_roles": [],
            "needs_clarification": true,
            "clarification_zh": "请提供分析目标（如洪涝风险评估、植被长势监测、土壤墒情、土地覆盖、热岛效应、森林火灾或地震滑坡背景）。",
        });
    }

    let per_role = limit_per_role.clamp(1, 5);
    let lower = trimmed.to_ascii_lowercase();
    let matched_recipe = TASK_BUNDLE_RECIPES
        .iter()
        .find(|r| r.markers.iter().any(|m| lower.contains(m)));

    let opts = CatalogSearchOptions {
        include_community,
        limit: (per_role * 4).max(8),
        force_refresh: false,
        include_deprecated: false,
        ..CatalogSearchOptions::default()
    };

    if let Some(recipe) = matched_recipe {
        let mut roles_map = serde_json::Map::new();
        let mut missing_roles: Vec<String> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        for &(role_key, role_label, role_query) in recipe.roles {
            let res = search_gee_catalog_with_options(app_data_dir, client, role_query, &opts);
            let mut picked: Vec<Value> = Vec::new();
            if let Some(items) = res["items"].as_array() {
                for item in items {
                    let id_l = item["id"].as_str().unwrap_or("").to_ascii_lowercase();
                    if id_l.is_empty() || !seen_ids.insert(id_l) {
                        continue;
                    }
                    picked.push(item.clone());
                    if picked.len() >= per_role {
                        break;
                    }
                }
            }
            if picked.is_empty() {
                missing_roles.push(role_key.to_owned());
            }
            roles_map.insert(
                role_key.to_owned(),
                json!({
                    "label_zh": role_label,
                    "candidates": picked,
                }),
            );
        }

        return json!({
            "ok": true,
            "dataset": "gee_recommend_datasets",
            "task": trimmed,
            "recipe": recipe.id,
            "recipe_title_zh": recipe.title_zh,
            "roles": Value::Object(roles_map),
            "missing_roles": missing_roles,
            "needs_clarification": !missing_roles.is_empty(),
            "selection_note_zh": "各角色候选为推荐起点组合，并非必须全部下载；编码或导出前请核验各数据集波段、尺度与时空覆盖。",
        });
    }

    let fallback = search_gee_catalog_with_options(app_data_dir, client, trimmed, &opts);
    let items: Vec<Value> = fallback["items"]
        .as_array()
        .map(|arr| arr.iter().take(per_role.max(3)).cloned().collect())
        .unwrap_or_default();
    let needs_clarification =
        items.is_empty() || fallback["needs_clarification"].as_bool() == Some(true);

    json!({
        "ok": true,
        "dataset": "gee_recommend_datasets",
        "task": trimmed,
        "recipe": "general",
        "recipe_title_zh": "通用主题配数推荐",
        "roles": {
            "primary": {
                "label_zh": "核心候选数据集",
                "candidates": items,
            }
        },
        "missing_roles": if items.is_empty() { vec!["primary"] } else { vec![] },
        "needs_clarification": needs_clarification,
        "clarification_zh": fallback["clarification_zh"],
        "selection_note_zh": "未命中预置科研多角色配方，已按关键词检索主候选；使用前请核验资产真实性与波段说明。",
    })
}

fn load_all_known_catalog_entries(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    include_community: bool,
) -> Vec<Value> {
    let opts = CatalogSearchOptions {
        include_community,
        limit: 100,
        force_refresh: false,
        include_deprecated: true,
        ..CatalogSearchOptions::default()
    };
    let mut rows: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for item in builtin_curated_datasets() {
        if !include_community && item.source == "community" {
            continue;
        }
        seen.insert(item.id.to_ascii_lowercase());
        let (deprecated, repl) = dataset_deprecation_info(&item.id);
        let (scale_note, qa_note) = curated_science_guidance(&item.id);
        let mut row = json!({
            "id": item.id,
            "title": item.title,
            "title_zh": item.title_zh,
            "source": item.source,
            "type": item.asset_type,
            "provider": item.provider,
            "start_date": item.start_date,
            "end_date": item.end_date,
            "native_scale_m": item.native_scale_m,
            "cloud_property": item.cloud_property,
            "default_bands": item.default_bands,
            "curated": true,
            "deprecated": deprecated,
            "scale_offset_note": scale_note,
            "qa_mask_note": qa_note,
            "verification_rule": verification_rule_for_source(&item.source, deprecated),
        });
        if let Some(r) = repl {
            row["replacement_id"] = json!(r);
        }
        rows.push(row);
    }

    let cache_dir = app_data_dir.join("gee_catalog_cache");
    let official_cache = cache_dir.join("official_catalog.json");
    let official_entries = if let Some(http) = client {
        ensure_cached_catalog_json(
            http,
            &official_cache,
            OFFICIAL_CATALOG_URL,
            opts.force_refresh,
        )
    } else if official_cache.is_file() {
        fs::read_to_string(&official_cache)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
    } else {
        None
    };

    if let Some(entries) = official_entries {
        for entry in entries {
            let id = entry["id"].as_str().unwrap_or("").trim();
            if id.is_empty() || !seen.insert(id.to_ascii_lowercase()) {
                continue;
            }
            let (deprecated, repl) = entry_is_deprecated(id, &entry);
            let mut row = json!({
                "id": id,
                "title": entry["title"].as_str().unwrap_or(id),
                "title_zh": "",
                "source": "official",
                "type": entry["type"].as_str().unwrap_or("image_collection"),
                "provider": entry["provider"].as_str().unwrap_or(""),
                "start_date": entry["start_date"].as_str().unwrap_or(""),
                "end_date": entry["end_date"].as_str().unwrap_or(""),
                "curated": false,
                "deprecated": deprecated,
                "verification_rule": verification_rule_for_source("official", deprecated),
            });
            if let Some(s) = parse_optional_scale_m(&entry) {
                row["native_scale_m"] = json!(s);
            }
            if let Some(r) = repl {
                row["replacement_id"] = json!(r);
            }
            rows.push(row);
        }
    }

    if include_community {
        let community_cache = cache_dir.join("community_catalog.json");
        let community_entries = if community_cache.is_file() {
            fs::read_to_string(&community_cache)
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        } else if let Some(http) = client {
            ensure_cached_catalog_json(http, &community_cache, COMMUNITY_CATALOG_URL, false)
        } else {
            None
        };
        if let Some(entries) = community_entries {
            for entry in entries {
                let id = entry["id"].as_str().unwrap_or("").trim();
                if id.is_empty() || !seen.insert(id.to_ascii_lowercase()) {
                    continue;
                }
                let (deprecated, repl) = entry_is_deprecated(id, &entry);
                let mut row = json!({
                    "id": id,
                    "title": entry["title"].as_str().unwrap_or(id),
                    "title_zh": "",
                    "source": "community",
                    "type": entry["type"].as_str().unwrap_or("image_collection"),
                    "provider": entry["provider"].as_str().unwrap_or(""),
                    "start_date": "",
                    "end_date": "",
                    "license": entry["license"].as_str().unwrap_or(""),
                    "curated": false,
                    "deprecated": deprecated,
                    "verification_rule": verification_rule_for_source("community", deprecated),
                });
                if let Some(s) = parse_optional_scale_m(&entry) {
                    row["native_scale_m"] = json!(s);
                }
                if let Some(r) = repl {
                    row["replacement_id"] = json!(r);
                }
                rows.push(row);
            }
        }
    }

    rows
}

fn find_catalog_record_by_id(rows: &[Value], asset_id: &str) -> Option<Value> {
    let trimmed = asset_id.trim();
    if trimmed.is_empty() {
        return None;
    }
    rows.iter()
        .find(|r| r["id"].as_str().unwrap_or("").eq_ignore_ascii_case(trimmed))
        .cloned()
        .or_else(|| {
            let (deprecated, repl) = dataset_deprecation_info(trimmed);
            if deprecated {
                let (scale_note, qa_note) = curated_science_guidance(trimmed);
                return Some(json!({
                    "id": trimmed,
                    "title": trimmed,
                    "title_zh": "",
                    "source": "official",
                    "type": "image_collection",
                    "provider": "",
                    "start_date": "",
                    "end_date": "",
                    "curated": false,
                    "deprecated": true,
                    "replacement_id": repl,
                    "scale_offset_note": scale_note,
                    "qa_mask_note": qa_note,
                    "verification_rule": verification_rule_for_source("official", true),
                }));
            }
            None
        })
}

pub fn compare_gee_datasets(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    ids: &[String],
    include_community: bool,
) -> Value {
    let mut unique_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in ids {
        let clean = raw.trim();
        if clean.is_empty() {
            continue;
        }
        if seen.insert(clean.to_ascii_lowercase()) {
            unique_ids.push(clean.to_owned());
            if unique_ids.len() >= 12 {
                break;
            }
        }
    }
    if unique_ids.len() < 2 {
        return json!({
            "ok": false,
            "dataset": "gee_compare_datasets",
            "error": "GEE_COMPARE_MIN_TWO_IDS",
            "message_zh": "对比数据集至少需要提供 2 个不同的有效 GEE 资产 ID。",
            "requested": unique_ids,
            "datasets": [],
            "missing": [],
        });
    }

    let catalog = load_all_known_catalog_entries(app_data_dir, client, include_community);
    let mut datasets = Vec::new();
    let mut missing = Vec::new();
    for req_id in &unique_ids {
        if let Some(found) = find_catalog_record_by_id(&catalog, req_id) {
            datasets.push(found);
        } else {
            missing.push(req_id.clone());
        }
    }

    json!({
        "ok": true,
        "dataset": "gee_compare_datasets",
        "requested": unique_ids,
        "datasets": datasets,
        "missing": missing,
        "comparison_fields": [
            "source",
            "type",
            "provider",
            "native_scale_m",
            "start_date",
            "end_date",
            "deprecated",
            "replacement_id",
            "verification_rule",
        ],
    })
}

fn validate_asset_id_for_lookup(asset_id: &str) -> Result<String, String> {
    let trimmed = asset_id.trim();
    if trimmed.is_empty() {
        return Err("GEE_ASSET_ID_EMPTY: 数据集 ID 不能为空".to_owned());
    }
    if trimmed.len() > 240
        || trimmed.contains("..")
        || trimmed.contains('?')
        || trimmed.contains('#')
        || trimmed.contains("://")
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("GEE_ASSET_ID_INVALID: 数据集 ID 包含非法字符或路径片段".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn sanitize_metadata_snippet(raw: &str, max_chars: usize) -> String {
    let cleaned = raw
        .replace("gs://", "[cloud-bucket] ")
        .replace("ee://", "[ee-asset] ")
        .replace("ftp://", "[ftp] ")
        .replace("https://", "")
        .replace("http://", "")
        .replace("?", " ")
        .replace("#", " ");
    let compact = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max_chars).collect()
}

pub fn verify_gee_dataset(
    app_data_dir: &Path,
    client: Option<&reqwest::blocking::Client>,
    asset_id: &str,
    live: bool,
) -> Result<Value, String> {
    let clean_id = validate_asset_id_for_lookup(asset_id)?;
    let catalog = load_all_known_catalog_entries(app_data_dir, None, true);
    let catalog_record = find_catalog_record_by_id(&catalog, &clean_id);
    let catalog_found = catalog_record.is_some();
    let deprecated = catalog_record
        .as_ref()
        .and_then(|r| r["deprecated"].as_bool())
        .unwrap_or_else(|| dataset_deprecation_info(&clean_id).0);
    let source = catalog_record
        .as_ref()
        .and_then(|r| r["source"].as_str())
        .unwrap_or(if clean_id.starts_with("projects/") {
            "community"
        } else {
            "official"
        })
        .to_owned();

    let mut stac_checked = false;
    let mut stac_found = false;
    let mut stac_band_names: Vec<String> = Vec::new();
    let mut stac_scale_m: Option<f64> = None;

    let mut live_checked = false;
    let mut live_exists: Option<bool> = None;
    let mut live_status = if live { "not_run" } else { "skipped" }.to_owned();
    let mut live_asset_summary: Option<Value> = None;

    if live {
        let api_base = super::gee_rest_api_base(app_data_dir);
        if let Some(http) = client {
            if api_base == "https://earthengine.googleapis.com"
                && !clean_id.starts_with("projects/")
                && clean_id.contains('/')
            {
                stac_checked = true;
                if let Ok(stac) = fetch_dataset_stac_bands(http, &clean_id) {
                    let bands: Vec<String> = stac["bands"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|b| b["name"].as_str().map(str::trim))
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    if stac.get("stac_url").is_some() || !bands.is_empty() {
                        stac_found = true;
                        stac_band_names = bands;
                        stac_scale_m = stac["native_scale_m"].as_f64();
                    }
                }
            }

            match super::resolve_gee_access_token(app_data_dir, http, None) {
                Ok((_proj, token)) => {
                    let asset_endpoint = if clean_id.starts_with("projects/") {
                        if clean_id.contains("/assets/") {
                            format!("{api_base}/v1/{clean_id}")
                        } else {
                            format!("{api_base}/v1/projects/earthengine-legacy/assets/{clean_id}")
                        }
                    } else {
                        format!("{api_base}/v1/projects/earthengine-public/assets/{clean_id}")
                    };
                    live_checked = true;
                    match http.get(&asset_endpoint).bearer_auth(&token).send() {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                live_exists = Some(true);
                                live_status = "verified".to_owned();
                                if let Ok(doc) = resp.json::<Value>() {
                                    let band_names: Vec<String> = doc["bands"]
                                        .as_array()
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|b| b["id"].as_str().map(str::trim))
                                                .filter(|s| !s.is_empty())
                                                .map(str::to_owned)
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    live_asset_summary = Some(json!({
                                        "id": doc["name"].as_str().unwrap_or(&clean_id),
                                        "type": doc["type"].as_str().unwrap_or(""),
                                        "start_time": doc["startTime"].as_str().unwrap_or(""),
                                        "end_time": doc["endTime"].as_str().unwrap_or(""),
                                        "band_names": band_names,
                                    }));
                                }
                            } else {
                                live_exists = Some(false);
                                live_status = format!("http_{}", resp.status().as_u16());
                            }
                        }
                        Err(_) => {
                            live_exists = None;
                            live_status = "network_error".to_owned();
                        }
                    }
                }
                Err(_) => {
                    live_checked = false;
                    live_exists = None;
                    live_status = "credentials_not_configured".to_owned();
                }
            }
        } else {
            live_status = "http_client_unavailable".to_owned();
        }
    }

    let status = if deprecated {
        "deprecated"
    } else if live_exists == Some(true) || (catalog_found && stac_found) {
        "verified"
    } else if live_checked && live_exists == Some(false) {
        "not_found"
    } else if catalog_found {
        "catalog_only"
    } else {
        "not_found"
    };

    let safe_summary = live_asset_summary.map(|mut s| {
        if let Some(title) = s["title"]
            .as_str()
            .map(|t| sanitize_metadata_snippet(t, 120))
        {
            s["title"] = json!(title);
        }
        s
    });

    Ok(json!({
        "ok": true,
        "dataset": "gee_verify_dataset",
        "id": clean_id,
        "source": source,
        "status": status,
        "deprecated": deprecated,
        "catalog_found": catalog_found,
        "catalog_record": catalog_record,
        "stac_checked": stac_checked,
        "stac_found": stac_found,
        "stac_scale_m": stac_scale_m,
        "stac_band_names": stac_band_names,
        "live_checked": live_checked,
        "live_exists": live_exists,
        "live_status": live_status,
        "live_asset": safe_summary,
        "verification_rule": verification_rule_for_source(&source, deprecated),
    }))
}

/// Fetch detailed STAC band metadata for a specific official GEE asset ID on demand.
pub fn fetch_dataset_stac_bands(
    client: &reqwest::blocking::Client,
    asset_id: &str,
) -> Result<Value, String> {
    let trimmed = asset_id.trim();
    if trimmed.is_empty() {
        return Err("GEE_ASSET_ID_EMPTY: 数据集 ID 不能为空".to_owned());
    }
    let curated = lookup_dataset_profile(trimmed);

    // Official GEE STAC path: COPERNICUS/S2_SR_HARMONIZED -> catalog/COPERNICUS/COPERNICUS_S2_SR_HARMONIZED.json
    if let Some((prefix, _)) = trimmed.split_once('/') {
        let flat = trimmed.replace('/', "_");
        let stac_url =
            format!("https://earthengine-stac.storage.googleapis.com/catalog/{prefix}/{flat}.json");
        if let Ok(resp) = client.get(&stac_url).send() {
            if resp.status().is_success() {
                if let Ok(doc) = resp.json::<Value>() {
                    let summaries = &doc["summaries"];
                    let bands = summaries["eo:bands"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let gsd = summaries["gsd"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(Value::as_f64)
                        .or_else(|| curated.as_ref().map(|c| c.native_scale_m))
                        .unwrap_or(30.0);
                    return Ok(json!({
                        "ok": true,
                        "id": trimmed,
                        "title": doc["title"].as_str().unwrap_or(trimmed),
                        "title_zh": curated.as_ref().map(|c| c.title_zh.as_str()).unwrap_or(""),
                        "description_snippet": doc["description"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(600)
                            .collect::<String>(),
                        "native_scale_m": gsd,
                        "cloud_property": curated.as_ref().and_then(|c| c.cloud_property.clone()),
                        "bands": bands,
                        "stac_url": stac_url,
                    }));
                }
            }
        }
    }

    if let Some(c) = curated {
        return Ok(json!({
            "ok": true,
            "id": c.id,
            "title": c.title,
            "title_zh": c.title_zh,
            "native_scale_m": c.native_scale_m,
            "cloud_property": c.cloud_property,
            "bands": c.default_bands.iter().map(|b| json!({"name": b})).collect::<Vec<_>>(),
            "asset_url": c.asset_url,
        }));
    }

    Ok(json!({
        "ok": true,
        "id": trimmed,
        "title": trimmed,
        "native_scale_m": 30.0,
        "bands": [],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eostage-cat-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("gee_catalog_cache")).unwrap();
        dir
    }

    #[test]
    fn test_lookup_dataset_profile_resolves_child_image_inside_curated_collection() {
        let p = lookup_dataset_profile(
            "COPERNICUS/S2_SR_HARMONIZED/20261015T025651_20261015T030519_T50SNA",
        )
        .expect("child image should inherit parent S2 collection profile");
        assert_eq!(p.asset_type, "image");
        assert!((p.native_scale_m - 10.0).abs() < 1e-6);
        assert!(p.default_bands.iter().any(|b| b == "B4"));
    }

    #[test]
    fn test_catalog_options_deprecation_resolution_and_clarification() {
        let dir = temp_dir("opts");
        fs::write(
            dir.join("gee_catalog_cache").join("official_catalog.json"),
            serde_json::to_string(&vec![
                json!({
                    "id": "COPERNICUS/S2_SR",
                    "title": "Sentinel-2 MSI Level-2A (Deprecated)",
                    "type": "image_collection",
                    "provider": "ESA",
                    "tags": "sentinel-2 optical",
                }),
                json!({
                    "id": "UNKNOWN/RES/CUSTOM_PRODUCT",
                    "title": "Sentinel-2 Custom Unknown Scale",
                    "type": "image_collection",
                    "provider": "ESA",
                    "tags": "sentinel-2 optical",
                }),
            ])
            .unwrap(),
        )
        .unwrap();

        // Default excludes deprecated COPERNICUS/S2_SR
        let no_dep = search_gee_catalog_with_options(
            &dir,
            None,
            "COPERNICUS/S2_SR",
            &CatalogSearchOptions::default(),
        );
        assert!(
            !no_dep["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|x| x["id"] == "COPERNICUS/S2_SR"),
            "deprecated entry must be excluded by default: {no_dep}"
        );

        // include_deprecated = true returns it flagged with replacement_id
        let with_dep = search_gee_catalog_with_options(
            &dir,
            None,
            "COPERNICUS/S2_SR",
            &CatalogSearchOptions {
                include_deprecated: true,
                ..CatalogSearchOptions::default()
            },
        );
        let dep_row = with_dep["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "COPERNICUS/S2_SR")
            .expect("deprecated row included when requested");
        assert_eq!(dep_row["deprecated"], true);
        assert_eq!(dep_row["replacement_id"], "COPERNICUS/S2_SR_HARMONIZED");

        // Invalid max_resolution_m <= 0 returns explicit error instead of empty pass-through
        let bad_res = search_gee_catalog_with_options(
            &dir,
            None,
            "sentinel",
            &CatalogSearchOptions {
                max_resolution_m: Some(0.0),
                ..CatalogSearchOptions::default()
            },
        );
        assert_eq!(bad_res["ok"], false);
        assert_eq!(bad_res["error"], "INVALID_MAX_RESOLUTION_M");

        // max_resolution_m = 30.0 excludes unknown-scale cached item and coarse items (> 30m)
        let res_30 = search_gee_catalog_with_options(
            &dir,
            None,
            "Sentinel-2",
            &CatalogSearchOptions {
                max_resolution_m: Some(30.0),
                ..CatalogSearchOptions::default()
            },
        );
        for row in res_30["items"].as_array().unwrap() {
            assert_ne!(row["id"], "UNKNOWN/RES/CUSTOM_PRODUCT");
            assert!(row["native_scale_m"].as_f64().unwrap() <= 30.0);
        }

        // Empty query never needs clarification; unmatched non-empty query needs clarification with 0 items
        let empty_q = search_gee_catalog(&dir, None, "", true, 10, false);
        assert_eq!(empty_q["needs_clarification"], false);
        assert!(empty_q["count"].as_u64().unwrap() > 0);

        let nomatch_q = search_gee_catalog(
            &dir,
            None,
            "不存在的火星地下暗物质传感器xyz",
            true,
            10,
            false,
        );
        assert_eq!(nomatch_q["count"], 0);
        assert_eq!(nomatch_q["needs_clarification"], true);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_recommend_compare_and_verify_workflows() {
        let dir = temp_dir("advisor");

        // 1. Role-based flood bundle deduplicates across 5 roles
        let rec = recommend_gee_dataset_bundle(&dir, None, "山区洪涝风险评估", 2, true);
        assert_eq!(rec["ok"], true);
        assert_eq!(rec["recipe"], "flood-risk");
        let roles = rec["roles"].as_object().unwrap();
        let mut seen = HashSet::new();
        for role in [
            "hazard_event",
            "water_baseline",
            "terrain",
            "rainfall",
            "exposure",
        ] {
            let cands = roles[role]["candidates"].as_array().unwrap();
            assert!(!cands.is_empty(), "missing candidates for role {role}");
            for c in cands {
                let id = c["id"].as_str().unwrap().to_owned();
                assert!(seen.insert(id.clone()), "duplicate recommended id {id}");
            }
        }

        // Empty task triggers clarification
        let empty_rec = recommend_gee_dataset_bundle(&dir, None, "   ", 2, true);
        assert_eq!(empty_rec["needs_clarification"], true);

        // 2. Compare datasets requires >= 2 unique IDs and resolves deprecated IDs
        let too_few = compare_gee_datasets(
            &dir,
            None,
            &[
                "COPERNICUS/S2_SR_HARMONIZED".into(),
                "copernicus/s2_sr_harmonized".into(),
            ],
            true,
        );
        assert_eq!(too_few["ok"], false);
        assert_eq!(too_few["error"], "GEE_COMPARE_MIN_TWO_IDS");

        let cmp = compare_gee_datasets(
            &dir,
            None,
            &[
                "COPERNICUS/S2_SR".into(),
                "COPERNICUS/S2_SR_HARMONIZED".into(),
                "NO/SUCH/DATASET".into(),
            ],
            true,
        );
        assert_eq!(cmp["ok"], true);
        assert_eq!(cmp["datasets"].as_array().unwrap().len(), 2);
        assert_eq!(cmp["missing"], json!(["NO/SUCH/DATASET"]));

        // 3. Verify dataset rejects unsafe IDs and degrades gracefully without credentials
        assert!(verify_gee_dataset(&dir, None, "", false).is_err());
        assert!(verify_gee_dataset(&dir, None, "../secret", false).is_err());
        assert!(verify_gee_dataset(&dir, None, "A/B?token=1", false).is_err());

        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let ver = verify_gee_dataset(&dir, Some(&http), "COPERNICUS/DEM/GLO30", true)
            .expect("verify without credentials must still succeed gracefully");
        assert_eq!(ver["ok"], true);
        assert_eq!(ver["catalog_found"], true);
        assert_eq!(ver["live_checked"], false);
        assert_eq!(ver["live_status"], "credentials_not_configured");

        let _ = fs::remove_dir_all(&dir);
    }
}
