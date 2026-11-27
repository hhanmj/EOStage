import { t } from "../../i18n/index.ts";

/**
 * 纯逻辑辅助函数：根据任务类别、数据集及名称映射数据类型分类键
 */
export function taskDataTypeKey(
  task:
    | {
        category?: string | null;
        dataset?: string | null;
        provider?: string | null;
        name?: string | null;
        task_type?: string | null;
      }
    | null
    | undefined
): string {
  if (!task) return "builtin:other";
  if (task.category === "dem_convert") {
    return "builtin:dem_convert";
  }

  const dataset = (task.dataset || "").trim();
  const name = (task.name || "").trim();
  const taskType = (task.task_type || "").trim().toLowerCase();
  const providerLower = (task.provider || "").trim().toLowerCase();

  if (
    providerLower.includes("gacos") ||
    dataset.toLowerCase().includes("gacos") ||
    name.toUpperCase().includes("GACOS")
  ) {
    return "GACOS";
  }

  if (
    providerLower.includes("orbit") ||
    taskType === "orbit" ||
    dataset.toLowerCase().includes("orbit") ||
    name.includes("精密轨道")
  ) {
    return "精密轨道 POEORB";
  }

  if (dataset) {
    const dsLower = dataset.toLowerCase();
    const nameUpper = name.toUpperCase();
    const dsUpper = dataset.toUpperCase();

    // 光学：内部数据集 ID（如 optical.landsat.c2.l2）不直接展示给用户
    if (dsLower.includes("landsat")) {
      return "Landsat";
    }
    if (dsLower.includes("hls")) {
      return "HLS";
    }
    if (dsLower.includes("sentinel-2") || dsLower.includes("sentinel2")) {
      return "Sentinel-2";
    }
    if (dsLower.includes("sentinel-3") || dsLower.includes("sentinel3") || dsLower.includes("sentinel_3")) {
      return "Sentinel-3";
    }
    if (dsLower.includes("sentinel-5p") || dsLower.includes("sentinel5p") || dsLower.includes("sentinel_5p")) {
      return "Sentinel-5P";
    }

    if (dsLower.includes("sentinel-1") || dsLower.includes("sentinel1") || dsLower.includes("s1")) {
      if (taskType === "sentinel1_grd" || dsUpper.includes("GRD") || nameUpper.includes("GRD")) {
        return "Sentinel-1 GRD";
      }
      if (taskType === "sentinel1_slc" || dsUpper.includes("SLC") || nameUpper.includes("SLC")) {
        return "Sentinel-1 SLC";
      }
      if (taskType === "sentinel1_raw" || dsUpper.includes("RAW") || nameUpper.includes("RAW")) {
        return "Sentinel-1 RAW";
      }
      if (taskType === "sentinel1_ocn" || dsUpper.includes("OCN") || nameUpper.includes("OCN")) {
        return "Sentinel-1 OCN";
      }
      return "Sentinel-1";
    }

    if (dsLower.includes("nisar")) {
      if (
        dsUpper.includes("NISAR L") ||
        dsLower.includes("nisar.l.") ||
        nameUpper.includes("NISAR L") ||
        name.includes("L波段") // i18n-exempt: matches backend task names, not display text
      ) {
        return "NISAR L";
      }
      if (
        dsUpper.includes("NISAR S") ||
        dsLower.includes("nisar.s.") ||
        dsLower.includes("ssar") ||
        nameUpper.includes("NISAR S") ||
        name.includes("S波段") // i18n-exempt: matches backend task names, not display text
      ) {
        return "NISAR S";
      }
      return "NISAR";
    }

    if (
      dsLower.includes("alos-2") ||
      dsLower.includes("alos2") ||
      dsLower.includes("palsar-2") ||
      dsLower.includes("palsar2") ||
      nameUpper.includes("ALOS-2")
    ) {
      return "ALOS-2";
    }
    if (dsLower.includes("alos") || dsLower.includes("palsar")) {
      return "ALOS PALSAR";
    }

    if (dsLower.includes("worldcover")) {
      return "ESA WorldCover";
    }
    if (dsLower.includes("jrc_gsw") || dsLower.includes("global_surface_water")) {
      return "JRC 地表水";
    }
    if (dsLower.includes("modis_ndvi") || dsLower.includes("mod13q1")) {
      return "MODIS NDVI";
    }
    if (dsLower.includes("ecostress")) {
      return "ECOSTRESS";
    }
    if (dsLower.includes("emit")) {
      return "NASA EMIT";
    }
    if (dsLower.includes("black_marble")) {
      return "NASA 黑夜灯光";
    }
    if (dsLower.includes("enmap")) {
      return "DLR EnMAP";
    }
    if (dsLower.includes("whu_clcd") || dsLower.includes("clcd")) {
      return "WHU CLCD";
    }
    if (dsLower.includes("tum_eot20") || dsLower.includes("eot20")) {
      return "TUM EOT20";
    }
    if (dsLower.includes("era5") || dsLower.includes("open_meteo")) {
      return "ERA5 气象";
    }
    if (dsLower.includes("noaa_aws_weather") || dsLower.includes("noaa_gfs") || dsLower.includes("noaa_goes")) {
      return "NOAA 气象";
    }

    if (
      dsLower === "bhoonidhi_sband_products" ||
      dsLower.includes("bhoonidhi")
    ) {
      return "NISAR S";
    }

    if (dsLower === "nisar_hdf5_subset") {
      if (
        nameUpper.includes("NISAR S") ||
        name.includes("S波段") ||
        name.toUpperCase().includes("S-BAND")
      ) {
        return "NISAR S";
      }
      return "NISAR";
    }

    if (
      providerLower.includes("compat.dem") ||
      dsLower.startsWith("dem.") ||
      dsLower === "open_dem_tiles" ||
      /^(cop30|cop90|fabdem|nasadem|srtmgl1|srtm_gl1|srtm_gl1_ellipsoidal|srtmgl3|srtm_gl3|aw3d30|aw3d30_ellipsoidal|aster_gdem|aster_gdem_v3|astgtm|gem15|gem15_dtm|gem15_dsm|nisar_dem|rema|arcticdem|etopo|noaa_etopo_2022|gebco|gebco_2026)$/i.test(
        dsLower
      )
    ) {
      return "DEM 高程";
    }

    if (dsLower.includes("gee") || dsLower.includes("earthengine")) {
      return "GEE 云端影像";
    }

    if (dsLower.includes("xyz") || dsLower.includes("tile")) {
      return "瓦片底图";
    }

    return dataset;
  }

  if (providerLower.includes("compat.dem")) {
    return "DEM 高程";
  }
  if (providerLower.includes("compat.sar")) {
    return "SAR 影像";
  }

  if (task.provider && task.provider.trim()) {
    return task.provider.trim();
  }

  return "builtin:other";
}

export function taskDataTypeLabel(typeKey: string): string {
  if (typeKey === "builtin:dem_convert") return t("downloads.filter.dem");
  if (typeKey === "builtin:other") return t("downloads.filter.other");
  return typeKey;
}

export function taskProductBadgeMeta(
  task:
    | {
        category?: string | null;
        dataset?: string | null;
        provider?: string | null;
        name?: string | null;
        task_type?: string | null;
      }
    | null
    | undefined
): { label: string; isSubset: boolean; modeLabel: string | null } {
  const baseLabel = taskDataTypeLabel(taskDataTypeKey(task));
  const taskType = (task?.task_type || "").trim().toLowerCase();
  const dsLower = (task?.dataset || "").trim().toLowerCase();
  if (taskType === "nisar_subset" || dsLower.includes("nisar_hdf5_subset")) {
    return {
      label: `${baseLabel} · 裁剪`,
      isSubset: true,
      modeLabel: "裁剪",
    };
  }
  if (taskType === "nisar_complete") {
    return {
      label: `${baseLabel} · 完整`,
      isSubset: false,
      modeLabel: "完整",
    };
  }
  return {
    label: baseLabel,
    isSubset: false,
    modeLabel: null,
  };
}

const CANONICAL_TASK_DISPLAY_NAME_MAP: Record<string, string> = {
  COP30: "Copernicus DEM 30m (GLO-30)",
  COP90: "Copernicus DEM 90m (GLO-90)",
  FABDEM: "FABDEM 30m 去林冠地面高程",
  NASADEM: "NASADEM 30m 全球高程",
  SRTMGL1: "SRTM GL1 30m 高程",
  SRTM_GL1: "SRTM GL1 30m 高程",
  SRTM_GL1_ELLIPSOIDAL: "SRTM GL1 椭球高 (30m)",
  SRTMGL3: "SRTM GL3 90m 高程",
  SRTM_GL3: "SRTM GL3 90m 高程",
  AW3D30: "ALOS World 3D 30m (AW3D30)",
  AW3D30_ELLIPSOIDAL: "ALOS World 3D 椭球高 (AW3D30)",
  ASTER_GDEM: "ASTER GDEM v3 30m",
  ASTER_GDEM_V3: "ASTER GDEM v3 30m",
  ASTGTM: "ASTER GDEM v3 30m",
  GEM15: "GEM-15 全球高程 (15m)",
  GEM15_DTM: "GEM-15 DTM 地形高程 (15m)",
  GEM15_DSM: "GEM-15 DSM 地表高程 (15m)",
  NISAR_DEM: "NISAR Copernicus DEM",
  GEBCO_2026: "GEBCO 2026 全球海洋水深地形",
  NOAA_ETOPO_2022: "NOAA ETOPO 2022 全球高程",
  ALOS_PALSAR_MOSAIC: "ALOS PALSAR 25m 镶嵌数据集",
  ESA_WORLDCOVER: "ESA WorldCover 土地覆盖",
  JRC_GSW: "JRC 全球地表水",
  MODIS_NDVI: "MODIS NDVI 植被指数",
  SENTINEL_5P: "Sentinel-5P 大气遥感",
  NASA_ECOSTRESS: "NASA ECOSTRESS 地表温度",
  NASA_EMIT: "NASA EMIT 高光谱",
  NASA_BLACK_MARBLE: "NASA 黑夜灯光 (Black Marble)",
  DLR_ENMAP: "DLR EnMAP 高光谱",
  WHU_CLCD: "WHU CLCD 中国年度土地覆盖",
  TUM_EOT20: "TUM EOT20 潮汐模型",
  OPEN_METEO_ERA5: "ERA5 再分析气象网格",
  NOAA_AWS_WEATHER: "NOAA 实时气象观测",
  BHOONIDHI_SBAND_PRODUCTS: "NISAR S-Band 产品",
  NISAR_HDF5_SUBSET: "NISAR AOI 在线裁剪",
  NISAR_PRODUCTS: "NISAR 多级别产品",
  GEE_CATALOG: "Google Earth Engine 云端数据",
  XYZ_SATELLITE_TILES: "全球高精度瓦片拼接",
  "OPTICAL.LANDSAT.C2.L2": "Landsat C2 L2 光学影像",
  "OPTICAL.SENTINEL-2.L2A": "Sentinel-2 L2A 光学影像",
  "OPTICAL.SENTINEL-2.L1C": "Sentinel-2 L1C 光学影像",
  "OPTICAL.SENTINEL-3.STANDARD": "Sentinel-3 标准光学产品",
  "SAR.SENTINEL-1.SLC": "Sentinel-1 SLC",
  "SAR.SENTINEL-1.GRD": "Sentinel-1 GRD",
  "SAR.NISAR.L.RSLC.PROVISIONAL.V1": "NISAR L-Band 暂行 RSLC",
  "SAR.NISAR.S.SSAR.RSLC.V1": "NISAR S-Band RSLC",
  "DEM.COP30.V1": "Copernicus DEM 30m (GLO-30)",
  "DEM.COP90.V1": "Copernicus DEM 90m (GLO-90)",
  "DEM.AW3D30-ELLIPSOIDAL.V1": "ALOS World 3D 椭球高 (AW3D30)",
  "DEM.SRTM-GL1-ELLIPSOIDAL.V1": "SRTM GL1 椭球高 (30m)",
  "DEM.ASTER-GDEM.V3": "ASTER GDEM v3 30m",
};

const CANONICAL_PROVIDER_LABEL_MAP: Record<string, string> = {
  "compat.dem": "Copernicus / OpenTopography",
  "compat.sar": "NASA ASF DAAC",
  "compat.optical": "光学卫星归档中心",
  "compat.gacos": "GACOS 官方申请",
  "gacos.webform": "GACOS 官方申请",
  isro_bhoonidhi: "ISRO Bhoonidhi",
  bhoonidhi: "ISRO Bhoonidhi",
  asf_cmr: "NASA ASF DAAC",
  mpc_science: "Microsoft Planetary Computer",
  nisar_subset: "NISAR 在线裁剪",
  gee_cloud: "Google Earth Engine",
  gee: "Google Earth Engine",
  xyz_tiles: "全球瓦片图源",
};

/**
 * 将底层原始数据集代号（如 COP30、bhoonidhi_sband_products、内部点号 ID）规范化为人类可读名称。
 */
export function normalizeTaskDisplayName(
  task:
    | {
        name?: string | null;
        dataset?: string | null;
      }
    | null
    | undefined
): string {
  const rawName = (task?.name || "").trim();
  if (!rawName) {
    const fallbackDs = (task?.dataset || "").trim();
    const mappedFallback = CANONICAL_TASK_DISPLAY_NAME_MAP[fallbackDs.toUpperCase()];
    return mappedFallback || fallbackDs || "未命名任务";
  }
  const direct = CANONICAL_TASK_DISPLAY_NAME_MAP[rawName.toUpperCase()];
  if (direct) return direct;
  if (isInternalDatasetId(rawName)) {
    const tail = rawName.split(".").pop()?.toUpperCase() || "";
    if (CANONICAL_TASK_DISPLAY_NAME_MAP[tail]) {
      return CANONICAL_TASK_DISPLAY_NAME_MAP[tail];
    }
  }
  return rawName;
}

/**
 * 规范化任务卡片来源提供方标签，隐藏 compat.* 等内部路由代号。
 */
export function normalizeTaskProviderLabel(provider: string | null | undefined): string {
  const raw = (provider || "").trim();
  if (!raw) return "";
  const mapped = CANONICAL_PROVIDER_LABEL_MAP[raw.toLowerCase()];
  if (mapped) return mapped;
  if (isInternalDatasetId(raw)) return "";
  return raw;
}

/**
 * 判断数据集字段是否为内部机器 ID（如 optical.landsat.c2.l2、sar.nisar.s.ssar.rslc.v1）。
 * 这类 ID 只用于路由与筛选，不应出现在任务卡片的摘要文案里。
 */
export function isInternalDatasetId(dataset: string | null | undefined): boolean {
  const value = (dataset || "").trim();
  return (
    /^[a-z0-9_-]+(\.[a-z0-9_-]+)+$/.test(value) ||
    /^[a-z][a-z0-9]*(_[a-z0-9]+)+$/.test(value)
  );
}

export function shouldHideRedundantDatasetToken(dataset: string | null | undefined): boolean {
  const value = (dataset || "").trim();
  if (!value || value === "—" || value === "-" || value === "--") return true;
  if (isInternalDatasetId(value)) return true;
  return Boolean(CANONICAL_TASK_DISPLAY_NAME_MAP[value.toUpperCase()]);
}

export type DownloadStatusFilter =
  | "all"
  | "active"
  | "paused"
  | "completed"
  | "failed";

export function taskMatchesStatusFilter(
  status: string | null | undefined,
  filter: DownloadStatusFilter
): boolean {
  if (filter === "all") return true;
  const s = (status || "").trim().toLowerCase();
  if (filter === "active") {
    return s === "running" || s === "queued";
  }
  if (filter === "paused") {
    return s === "paused";
  }
  if (filter === "completed") {
    return s === "completed";
  }
  return s === "failed" || s === "cancelled" || s === "interrupted";
}
