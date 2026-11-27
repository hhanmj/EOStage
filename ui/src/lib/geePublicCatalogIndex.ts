export type GeeCatalogTheme =
  | "all"
  | "optical"
  | "sar"
  | "dem"
  | "landcover"
  | "water"
  | "climate"
  | "ecology"
  | "atmosphere";

export type GeeAssetKind = "IMAGE" | "IMAGE_COLLECTION";
export type GeeFilterTemplate = "optical" | "sar" | "climate_env" | "static";

export interface GeeBandChoice {
  id: string;
  label: string;
}

export interface GeeCatalogThemeDef {
  id: GeeCatalogTheme;
  label: string;
}

export interface GeePublicDatasetEntry {
  assetId: string;
  title: string;
  subtitle: string;
  theme: Exclude<GeeCatalogTheme, "all">;
  resolution: string;
  era: string;
  provider: string;
  assetKind: GeeAssetKind;
  filterTemplate: GeeFilterTemplate;
  cloudProp?: string;
  selectableBands: GeeBandChoice[];
  coverageNote?: string;
  presetResourceId?: string;
  tags: string[];
}

export const GEE_CATALOG_THEMES: GeeCatalogThemeDef[] = [
  { id: "all", label: "全部" },
  { id: "optical", label: "多光谱光学" },
  { id: "sar", label: "SAR 雷达" },
  { id: "dem", label: "高程地形" },
  { id: "landcover", label: "土地覆盖" },
  { id: "water", label: "地表水体" },
  { id: "climate", label: "气象降水" },
  { id: "ecology", label: "植被生态" },
  { id: "atmosphere", label: "大气夜光" },
];

/**
 * 精选 24 个 GEE 高价值云端计算与独占遥感数据集索引：
 * 精简剔除低效冗余重复条目，保留云端指数/去云合成与 GEE 独占产品，并附带真实资产类型与动态筛选模板。
 */
export const GEE_PUBLIC_CATALOG_INDEX: GeePublicDatasetEntry[] = [
  // 多光谱光学 (4)
  {
    assetId: "COPERNICUS/S2_SR_HARMONIZED",
    title: "Sentinel-2 L2A 地表反射率 (云端去云合成)",
    subtitle: "10m MSI 多光谱去大气反射率，适合 NDVI/MNDWI 与中值无云拼接",
    theme: "optical",
    resolution: "10m",
    era: "2017–至今",
    provider: "ESA / Copernicus",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "optical",
    cloudProp: "CLOUDY_PIXEL_PERCENTAGE",
    selectableBands: [
      { id: "", label: "默认真彩 RGB (B4/B3/B2)" },
      { id: "B4,B3,B2", label: "真彩 RGB (B4,B3,B2)" },
      { id: "B8,B4,B3", label: "假彩色植被 (B8,B4,B3)" },
      { id: "B8,B11,B4", label: "水分与农林分析 (B8,B11,B4)" },
    ],
    presetResourceId: "science.gee.s2-sr.v1",
    tags: ["sentinel2", "s2", "optical", "ndvi", "mndwi", "l2a"],
  },
  {
    assetId: "LANDSAT/LC09/C02/T1_L2",
    title: "Landsat 9 C2-L2 地表反射率与温度",
    subtitle: "OLI-2 / TIRS-2 30m 光学与热红外长时序拼接",
    theme: "optical",
    resolution: "30m",
    era: "2021–至今",
    provider: "USGS",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "optical",
    cloudProp: "CLOUD_COVER",
    selectableBands: [
      { id: "", label: "默认真彩 RGB (SR_B4/B3/B2)" },
      { id: "SR_B4,SR_B3,SR_B2", label: "真彩 RGB (SR_B4/B3/B2)" },
      { id: "SR_B5,SR_B4,SR_B3", label: "标准假彩色 (SR_B5/B4/B3)" },
      { id: "ST_B10", label: "热红外地表温度 (ST_B10)" },
    ],
    presetResourceId: "science.gee.landsat9.v1",
    tags: ["landsat", "landsat9", "lc09", "optical", "thermal", "lst"],
  },
  {
    assetId: "LANDSAT/LC08/C02/T1_L2",
    title: "Landsat 8 C2-L2 地表反射率与温度",
    subtitle: "2013 至今 30m 连续光学/热红外云端长序列",
    theme: "optical",
    resolution: "30m",
    era: "2013–至今",
    provider: "USGS",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "optical",
    cloudProp: "CLOUD_COVER",
    selectableBands: [
      { id: "", label: "默认真彩 RGB (SR_B4/B3/B2)" },
      { id: "SR_B4,SR_B3,SR_B2", label: "真彩 RGB (SR_B4/B3/B2)" },
      { id: "SR_B5,SR_B4,SR_B3", label: "标准假彩色 (SR_B5/B4/B3)" },
      { id: "ST_B10", label: "热红外地表温度 (ST_B10)" },
    ],
    presetResourceId: "science.gee.landsat8.v1",
    tags: ["landsat", "landsat8", "lc08", "optical", "thermal"],
  },
  {
    assetId: "LANDSAT/LT05/C02/T1_L2",
    title: "Landsat 5 TM C2-L2 历史长时序",
    subtitle: "1984–2012 经典 28 年 30m 历史光学回溯数据源",
    theme: "optical",
    resolution: "30m",
    era: "1984–2012",
    provider: "USGS",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "optical",
    cloudProp: "CLOUD_COVER",
    selectableBands: [
      { id: "", label: "默认真彩 RGB (SR_B3/B2/B1)" },
      { id: "SR_B3,SR_B2,SR_B1", label: "真彩 RGB (SR_B3/B2/B1)" },
      { id: "SR_B4,SR_B3,SR_B2", label: "假彩色植被 (SR_B4/B3/B2)" },
    ],
    tags: ["landsat", "landsat5", "lt05", "tm", "historical"],
  },

  // SAR 雷达 (2)
  {
    assetId: "COPERNICUS/S1_GRD",
    title: "Sentinel-1 C 波段 SAR GRD",
    subtitle: "10m 全天候穿透云雨后向散射强度 (Sigma0)",
    theme: "sar",
    resolution: "10m",
    era: "2014–至今",
    provider: "ESA / Copernicus",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "sar",
    selectableBands: [
      { id: "", label: "默认 VV + VH 极化" },
      { id: "VV,VH", label: "双极化 (VV, VH)" },
      { id: "VV", label: "单极化 (VV)" },
      { id: "VH", label: "交叉极化 (VH)" },
    ],
    presetResourceId: "science.gee.s1-grd.v1",
    tags: ["sentinel1", "sar", "grd", "vv", "vh", "radar"],
  },
  {
    assetId: "JAXA/ALOS/PALSAR/YEARLY/SAR_EPOCH",
    title: "ALOS PALSAR L 波段年度雷达拼图",
    subtitle: "25m L 波段穿透林冠 HH/HV 雷达年际镶嵌",
    theme: "sar",
    resolution: "25m",
    era: "2007–至今",
    provider: "JAXA EORC",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "sar",
    selectableBands: [
      { id: "", label: "默认 HH + HV" },
      { id: "HH,HV", label: "双极化 (HH, HV)" },
      { id: "HH", label: "同极化 (HH)" },
      { id: "HV", label: "交叉极化 (HV)" },
    ],
    tags: ["alos", "palsar", "l-band", "sar"],
  },

  // 高程地形 (5)
  {
    assetId: "projects/climate-and-sustainability/assets/gem-15_v1",
    title: "Google GEM-15 15m 高程 (DTM + DSM)",
    subtitle: "多源立体光测融合 15m EGM96 正高 · 官方 v1 不含中国大陆及极地",
    theme: "dem",
    resolution: "15m",
    era: "2018–2025",
    provider: "Google Research",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "static",
    selectableBands: [
      { id: "", label: "默认裸地 DTM + 地表 DSM" },
      { id: "dtm", label: "裸地地形 (dtm)" },
      { id: "dsm", label: "地表高程 (dsm)" },
    ],
    coverageNote: "官方 v1 不含中国大陆、俄罗斯、蒙古、朝鲜及南北极；覆盖区内可按 15m 原生导出",
    presetResourceId: "science.gee.gem-15.v1",
    tags: ["gem-15", "gem15", "google", "15m", "dem", "dtm", "dsm"],
  },
  {
    assetId: "projects/sat-io/open-datasets/FABDEM",
    title: "FABDEM 30m 去除建筑与植被裸地高程",
    subtitle: "基于 Copernicus GLO-30 机器学习剔除森林与建筑偏移的 30m 裸地 DEM",
    theme: "dem",
    resolution: "30m",
    era: "基准产品",
    provider: "Univ. of Bristol / GEE",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "static",
    selectableBands: [{ id: "b1", label: "裸地高程 (b1)" }],
    tags: ["fabdem", "bare earth", "dem", "flood"],
  },
  {
    assetId: "COPERNICUS/DEM/GLO30",
    title: "Copernicus DEM GLO-30 全球高程",
    subtitle: "TanDEM-X 全球 30m 高精度正高基准",
    theme: "dem",
    resolution: "30m",
    era: "2011–2015",
    provider: "Airbus / ESA",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "static",
    selectableBands: [{ id: "DEM", label: "高程值 (DEM)" }],
    presetResourceId: "science.gee.dem-glo30.v1",
    tags: ["copernicus", "glo30", "dem", "insar"],
  },
  {
    assetId: "NASA/NASADEM_HGT/001",
    title: "NASA NASADEM 全球 30m 高程",
    subtitle: "重处理 SRTM 融合 ICESat 与 ASTER 去空洞基准高程",
    theme: "dem",
    resolution: "30m",
    era: "2000",
    provider: "NASA JPL",
    assetKind: "IMAGE",
    filterTemplate: "static",
    selectableBands: [{ id: "elevation", label: "高程值 (elevation)" }],
    presetResourceId: "science.gee.nasadem.v1",
    tags: ["nasadem", "srtm", "dem"],
  },
  {
    assetId: "USGS/SRTMGL1_003",
    title: "SRTM Plus 全球 30m 数字高程 (V3)",
    subtitle: "±60° 纬度区间经典 1 弧秒地形基准",
    theme: "dem",
    resolution: "30m",
    era: "2000",
    provider: "NASA / USGS",
    assetKind: "IMAGE",
    filterTemplate: "static",
    selectableBands: [{ id: "elevation", label: "高程值 (elevation)" }],
    tags: ["srtm", "dem", "30m"],
  },

  // 土地覆盖 (4)
  {
    assetId: "GOOGLE/DYNAMICWORLD/V1",
    title: "Dynamic World 10m 近实时土地利用",
    subtitle: "9 大类地物深度学习逐景概率，支持按时段众数/中值合成",
    theme: "landcover",
    resolution: "10m",
    era: "2015–至今",
    provider: "Google / WRI",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [
      { id: "label", label: "主类别标签 (label)" },
      { id: "built", label: "建成区概率 (built)" },
      { id: "water", label: "水体概率 (water)" },
      { id: "trees", label: "林木概率 (trees)" },
      { id: "crops", label: "耕地概率 (crops)" },
    ],
    presetResourceId: "science.gee.dynamic-world.v1",
    tags: ["dynamic world", "lulc", "10m", "landcover"],
  },
  {
    assetId: "ESA/WorldCover/v200",
    title: "ESA WorldCover v200 10m 土地覆盖",
    subtitle: "欧空局 11 类 2021 全球 10 米精细地物分类基准",
    theme: "landcover",
    resolution: "10m",
    era: "2021",
    provider: "ESA",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "static",
    selectableBands: [{ id: "Map", label: "地物分类码 (Map)" }],
    presetResourceId: "science.gee.worldcover.v1",
    tags: ["worldcover", "v200", "lulc"],
  },
  {
    assetId: "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M",
    title: "GHSL 全球城市建筑高度 100m",
    subtitle: "欧盟 JRC 全球建筑平均/总高度形态栅格",
    theme: "landcover",
    resolution: "100m",
    era: "2018",
    provider: "European Commission JRC",
    assetKind: "IMAGE",
    filterTemplate: "static",
    selectableBands: [{ id: "b1", label: "建筑高度 (b1)" }],
    tags: ["ghsl", "urban", "building", "height"],
  },
  {
    assetId: "projects/sat-io/open-datasets/CLCD",
    title: "武汉大学中国逐年 30m 土地覆盖 CLCD (社区精选)",
    subtitle: "杨杰/黄昕团队 1985–2023 中国逐年 9 类 30m 土地覆盖基准数据集",
    theme: "landcover",
    resolution: "30m",
    era: "1985–2023",
    provider: "Wuhan University",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "static",
    selectableBands: [{ id: "b1", label: "土地覆盖分类码 (b1)" }],
    tags: ["clcd", "china", "landcover", "lulc", "武汉大学", "土地覆盖", "社区"],
  },

  // 地表水体 (2)
  {
    assetId: "JRC/GSW1_4/GlobalSurfaceWater",
    title: "JRC 全球地表水体动态分布 (综合)",
    subtitle: "30m 水体出现频率、季节性与历史变迁汇总",
    theme: "water",
    resolution: "30m",
    era: "1984–2021",
    provider: "European Commission JRC",
    assetKind: "IMAGE",
    filterTemplate: "static",
    selectableBands: [
      { id: "occurrence", label: "水体出现频率 (occurrence)" },
      { id: "seasonality", label: "年内水体季节性 (seasonality)" },
      { id: "max_extent", label: "历史最大水域 (max_extent)" },
      { id: "change_abs", label: "水体绝对变化 (change_abs)" },
    ],
    presetResourceId: "science.gee.gsw.v1",
    tags: ["gsw", "jrc", "water", "flood"],
  },
  {
    assetId: "JRC/GSW1_4/MonthlyHistory",
    title: "JRC 全球地表水体月度历史 (1984–2021)",
    subtitle: "30m 逐月永久/季节性水体分布序列，适合洪水演变时序统计",
    theme: "water",
    resolution: "30m",
    era: "1984–2021",
    provider: "European Commission JRC",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [{ id: "water", label: "月度水体状态 (water)" }],
    tags: ["gsw", "monthly", "water", "flood"],
  },

  // 气象降水与再分析 (3)
  {
    assetId: "ECMWF/ERA5_LAND/HOURLY",
    title: "ECMWF ERA5-Land 陆面气象再分析",
    subtitle: "~9km 气温、地表气压、总降水与土壤水长序列格网",
    theme: "climate",
    resolution: "~9km",
    era: "1950–至今",
    provider: "ECMWF / Copernicus",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [
      { id: "temperature_2m", label: "2米气温 (temperature_2m)" },
      { id: "total_precipitation_hourly", label: "逐小时降水 (total_precipitation_hourly)" },
      { id: "surface_pressure", label: "地表气压 (surface_pressure)" },
      { id: "volumetric_soil_water_layer_1", label: "表层土壤含水量" },
    ],
    presetResourceId: "science.gee.era5-land.v1",
    tags: ["era5", "hourly", "temperature", "precipitation"],
  },
  {
    assetId: "UCSB-CHG/CHIRPS/DAILY",
    title: "CHIRPS v2 逐日高分辨率降水",
    subtitle: "5.5km 卫星红外与站点融合降水长序列",
    theme: "climate",
    resolution: "~5.5km",
    era: "1981–至今",
    provider: "UC Santa Barbara CHG",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [{ id: "precipitation", label: "日降水量 (precipitation, mm)" }],
    tags: ["chirps", "daily", "precipitation", "drought"],
  },
  {
    assetId: "NASA/GPM_L3/IMERG_V07",
    title: "NASA GPM IMERG V07 全球卫星降水",
    subtitle: "0.1° 半小时近实时降水监测（暴雨滑坡预警常用）",
    theme: "climate",
    resolution: "0.1° (~11km)",
    era: "2000–至今",
    provider: "NASA GES DISC",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [{ id: "precipitation", label: "降水强度 (precipitation, mm/hr)" }],
    tags: ["gpm", "imerg", "precipitation", "rainfall"],
  },

  // 植被生态 (3)
  {
    assetId: "MODIS/061/MOD13Q1",
    title: "MODIS Terra 250m 16天植被指数",
    subtitle: "NDVI / EVI 生态退化与物候监测经典产品",
    theme: "ecology",
    resolution: "250m",
    era: "2000–至今",
    provider: "NASA LP DAAC",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [
      { id: "NDVI", label: "归一化植被指数 (NDVI)" },
      { id: "EVI", label: "增强植被指数 (EVI)" },
    ],
    presetResourceId: "science.gee.modis-ndvi.v1",
    tags: ["mod13q1", "ndvi", "evi", "vegetation"],
  },
  {
    assetId: "MODIS/061/MOD11A1",
    title: "MODIS Terra 每日 1km 地表温度 (LST)",
    subtitle: "昼夜地表热异常与热岛效应监测",
    theme: "ecology",
    resolution: "1km",
    era: "2000–至今",
    provider: "NASA LP DAAC",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [
      { id: "LST_Day_1km", label: "白天地面温度 (LST_Day_1km)" },
      { id: "LST_Night_1km", label: "夜间地面温度 (LST_Night_1km)" },
    ],
    tags: ["lst", "temperature", "mod11a1", "thermal"],
  },
  {
    assetId: "UMD/hansen/global_forest_change_2023_v1_11",
    title: "Hansen 全球 30m 森林覆盖与损毁变化",
    subtitle: "2000–2023 逐年森林砍伐年份与基准树冠覆盖度",
    theme: "ecology",
    resolution: "30m",
    era: "2000–2023",
    provider: "University of Maryland",
    assetKind: "IMAGE",
    filterTemplate: "static",
    selectableBands: [
      { id: "loss", label: "森林损毁掩膜 (loss)" },
      { id: "lossyear", label: "损毁发生年份 (lossyear)" },
      { id: "treecover2000", label: "2000 年树冠覆盖率 (treecover2000)" },
      { id: "gain", label: "森林新生掩膜 (gain)" },
    ],
    tags: ["hansen", "forest", "loss", "deforestation"],
  },

  // 大气夜光 (2)
  {
    assetId: "COPERNICUS/S5P/OFFL/L3_NO2",
    title: "Sentinel-5P 对流层二氧化氮 (NO2)",
    subtitle: "大气污染排放与区域空气质量监测",
    theme: "atmosphere",
    resolution: "1.1km",
    era: "2018–至今",
    provider: "ESA / KNMI",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [
      { id: "tropospheric_NO2_column_number_density", label: "对流层 NO2 柱浓度" },
      { id: "NO2_column_number_density", label: "总 NO2 柱浓度" },
    ],
    presetResourceId: "science.gee.s5p-no2.v1",
    tags: ["s5p", "no2", "air quality", "atmosphere"],
  },
  {
    assetId: "NOAA/VIIRS/DNB/MONTHLY_V1/VCMSLCFG",
    title: "VIIRS DNB 月度夜间灯光 (杂散光校正)",
    subtitle: "460m 区域经济活力、停电监测与光污染研究",
    theme: "atmosphere",
    resolution: "~460m",
    era: "2014–至今",
    provider: "Earth Observation Group",
    assetKind: "IMAGE_COLLECTION",
    filterTemplate: "climate_env",
    selectableBands: [{ id: "avg_rad", label: "平均夜间辐射亮度 (avg_rad)" }],
    tags: ["viirs", "dnb", "nightlight", "夜光"],
  },
];

const STATIC_GEE_ASSET_IDS = new Set([
  "COPERNICUS/DEM/GLO30",
  "NASA/NASADEM_HGT/001",
  "USGS/SRTMGL1_003",
  "ESA/WorldCover/v200",
  "ESA/WorldCover/v100",
  "JRC/GSW1_4/GlobalSurfaceWater",
  "CSP/ERGo/1_0/Global/ALOS_landforms",
  "projects/climate-and-sustainability/assets/gem-15_v1",
  "projects/sat-io/open-datasets/FABDEM",
  "projects/sat-io/open-datasets/GHSL/GHSL_BUILT_H_AGBH_2018_100M",
  "UMD/hansen/global_forest_change_2023_v1_11",
]);

export function lookupGeePublicDataset(
  assetId?: string | null
): GeePublicDatasetEntry | undefined {
  const clean = (assetId ?? "").trim().toLowerCase();
  if (!clean || clean === "all") return undefined;
  return GEE_PUBLIC_CATALOG_INDEX.find((d) => d.assetId.toLowerCase() === clean);
}

/**
 * 判断 GEE Asset 是否为静态无时相数据集（DEM / 土地覆被 / 综合水体 / 森林基线），
 * 检索或计算时应隐藏并跳过日期与云量过滤。
 */
export function isStaticGeeAssetId(assetId?: string | null): boolean {
  const trimmed = (assetId ?? "").trim();
  if (!trimmed || trimmed.toUpperCase() === "ALL") return false;
  const entry = lookupGeePublicDataset(trimmed);
  if (entry) {
    return entry.filterTemplate === "static";
  }
  if (STATIC_GEE_ASSET_IDS.has(trimmed)) return true;
  const upper = trimmed.toUpperCase();
  return (
    upper.includes("/DEM/") ||
    upper.includes("COPERNICUS/DEM") ||
    upper.includes("FABDEM") ||
    upper.includes("AW3D30") ||
    upper.includes("NASADEM") ||
    upper.includes("SRTM") ||
    upper.includes("MERIT/DEM") ||
    upper.includes("ESA/WORLDCOVER") ||
    upper.includes("JRC/GSW1_4/GLOBALSURFACEWATER") ||
    upper.includes("JRC/GHSL") ||
    upper.includes("GHSL_BUILT") ||
    upper.includes("GLOBAL_FOREST_CHANGE") ||
    upper.includes("GEM-15") ||
    upper.includes("ALOS_LANDFORMS")
  );
}

export function filterGeePublicCatalog(
  theme: GeeCatalogTheme,
  query?: string
): GeePublicDatasetEntry[] {
  const cleanQuery = (query ?? "").trim().toLowerCase();
  return GEE_PUBLIC_CATALOG_INDEX.filter((entry) => {
    if (theme !== "all" && entry.theme !== theme) return false;
    if (!cleanQuery) return true;
    const hay = [
      entry.assetId,
      entry.title,
      entry.subtitle,
      entry.provider,
      entry.resolution,
      ...entry.tags,
    ]
      .join(" ")
      .toLowerCase();
    return hay.includes(cleanQuery);
  });
}
