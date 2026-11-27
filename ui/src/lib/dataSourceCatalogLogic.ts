/**
 * dataSourceCatalogLogic.ts
 * 数据源目录核心纯逻辑模块
 * 事实优先规范：只展示经权威注册且后端真实支持检索/下载的数据源。
 */

import type { SourceDescriptor } from "../shell/api/types.ts";
import { getStorage } from "./webStorage.ts";
import { isSourceSearchOnly } from "../shell/api/sourceConditions.ts";
import { getCurrentLocale } from "../i18n/index.ts";
import type { Locale } from "../i18n/types.ts";
import {
  applyCatalogItemLocale,
  CATALOG_ITEM_TRANSLATIONS,
  localizeCatalogDynamicAccountText,
} from "./dataSourceCatalogTranslations.ts";
import catalogRegistryJson from "./dataSourceCatalogRegistry.json" with { type: "json" };

export type CatalogCategoryKey =
  | "recent"
  | "primary-all"
  | "primary-sar"
  | "primary-optical"
  | "primary-dem"
  | "primary-thematic"
  | "auxiliary-all"
  | "auxiliary-orbit"
  | "auxiliary-atmospheric"
  | "auxiliary-geodesy"
  | "gee-catalog";

export type CatalogBandTag = "all" | "C" | "L" | "S" | "X" | "optical";
export type CatalogAccountTag = "all" | "free" | "account_required";

export type CatalogItemStatus = "available" | "search_only" | "account_required" | "in_progress";

export interface CatalogItemMetadata {
  name: string;
  subtitle: string;
  band?: string;
  bandTag?: "C" | "L" | "S" | "X" | "optical" | "dem" | "other";
  era?: string;
  provider?: string;
  group: "orbiting" | "historical" | "dem" | "auxiliary";
  category: CatalogCategoryKey;
  subcategories: CatalogCategoryKey[];
  defaultStatus: CatalogItemStatus;
  supportsOnlineCropping?: boolean;
  requiredAccountIds?: string[];
  description: string;
  satellites?: string;
  timeRange?: string;
  accountRequirement: "free" | "account_required";
  accountText: string;
  filters: string[];
  companionDownloads?: string[];
  usageNotes?: string[];
  geeAssetId?: string;
}

export interface CatalogItem extends CatalogItemMetadata {
  resourceId: string;
  status: CatalogItemStatus;
  disabled: boolean;
}

export const RECENT_CATALOG_SOURCES_STORAGE_KEY = "sp_recent_catalog_sources";
export const MAX_RECENT_SOURCES = 8;

/**
 * 已下架或仅限本地转换的内部资源 ID，不在在线数据源目录中展示
 */
export const RETIRED_OR_LOCAL_RESOURCE_IDS: ReadonlySet<string> = new Set([
  "dem.user-local.v1",
  "science.ers-1-2.sar.v1",
  "science.jers-1.sar.v1",
  "science.radarsat-1.sar.v1",
  "science.envisat.reserved.v1",
  "science.envisat.asar.v1",
]);

/**
 * 同一传感器家族已在条件面板提供处理级别切换的子级别入口，目录中合并为单一家族条目，
 * 但保留 `CATALOG_METADATA_REGISTRY` 元数据供条件面板/下载能力查询。
 */
export const COLLAPSED_FAMILY_SUBLEVEL_IDS: ReadonlySet<string> = new Set([
  "science.sentinel-2.l1c.v1",
]);

export const CATALOG_METADATA_REGISTRY: Record<string, CatalogItemMetadata> =
  catalogRegistryJson as unknown as Record<string, CatalogItemMetadata>;

const CANONICAL_SHORT_NAMES: Record<string, string> = {
  "science.nisar.l.rslc.provisional.v1": "NISAR L 波段 SAR",
  "science.nisar.s.ssar.rslc.v1": "NISAR S 波段 SAR",
  "science.sentinel-1.standard.v1": "Sentinel-1",
  "science.sentinel-1.slc-burst.v1": "Sentinel-1 SLC Burst",
  "science.alos-2.palsar-2.scansar-l1.1.v1": "ALOS-2 PALSAR-2 ScanSAR",
  "science.alos.palsar.v1": "ALOS PALSAR",
  "science.alos.palsar.l1.0-1.5.v1": "ALOS PALSAR",
  "science.seasat.sar.v1": "SEASAT",
  "science.sentinel-2.l2a.v1": "Sentinel-2 L2A",
  "science.sentinel-2.l1c.v1": "Sentinel-2 L1C",
  "science.landsat.c2-l2.v1": "Landsat 4-9 C2-L2",
  "science.hls.l30.v1": "HLS L30 (Landsat)",
  "science.hls.l30.v2": "HLS L30 (Landsat)",
  "science.hls.s30.v1": "HLS S30 (Sentinel-2)",
  "science.hls.s30.v2": "HLS S30 (Sentinel-2)",
  "dem.cop30.v1": "Copernicus DEM GLO-30（30米）",
  "dem.cop90.v1": "Copernicus DEM GLO-90（90米）",
  "dem.nasadem.v1": "NASADEM Global 30m（30米）",
  "dem.aw3d30.v1": "ALOS World 3D 30m（正高 · 30米）",
  "dem.aw3d30-ellipsoidal.v1": "ALOS World 3D 30m（椭球高 · 30米）",
  "dem.nisar-dem.v1": "NISAR 科学参考 DEM（椭球高 · 30米）",
  "dem.fabdem.v1": "FABDEM V1-2 30m（裸地正高 · 30米）",
  "dem.gem15-dtm.v1": "Google GEM-15 DTM 15m（裸地 · 15米）",
  "dem.gem15-dsm.v1": "Google GEM-15 DSM 15m（地表 · 15米）",
  "dem.srtm-gl1.v1": "SRTM GL1 Global 30m（正高 · 30米）",
  "dem.srtm-gl1-ellipsoidal.v1": "SRTM GL1 Global 30m（椭球高 · 30米）",
  "dem.srtm-gl3.v1": "SRTM GL3 Global 90m（90米）",
  "dem.aster-gdem.v3": "ASTER GDEM v3 Global 30m（30米）",
  "science.opera.disp-s1.v1": "NASA OPERA 地表位移与配准 (DISP-S1 / CSLC)",
  "science.comet.licsar.v1": "COMET-LiCSAR 全球干涉图与相干图",
  "atmospheric.era5.reanalysis.v1": "ERA5 再分析大气延迟数据",
  "atmospheric.gacos.ztd.v1": "GACOS 大气延迟改正",
  "orbit.sentinel-1.poeorb.v1": "Sentinel-1 精密轨道 (POEORB/RESORB)",
  "orbit.nisar.oe.v1": "NISAR 精密/标称轨道历书 (POE/MOE/NOE/FOE)",
  "orbit.copernicus.pod.v1": "Sentinel-2 / Sentinel-3 精密轨道 (POD POEORB)",
  "ionospheric.igs.gim.v1": "IGS / CODE 全球电离层延迟格网 (IONEX GIM)",
  "event.usgs.earthquake.v1": "USGS 全球地震事件目录与同震定位",
  "geodesy.ngl.gnss.v1": "NGL 全球 GNSS 基准站位移时间序列",
  "auxiliary.xyz-tiles.v1": "全球高清卫星与底图瓦片 (XYZ Tiles)",
  "science.esa.worldcover.v1": "ESA WorldCover 全球 10 米土地利用分类",
  "science.jrc.gsw.v1": "JRC 全球地表水体动态分布 (GSW)",
  "science.alos.palsar-mosaic.v1": "ALOS PALSAR 全球 25 米年度雷达拼图",
  "science.modis.ndvi.v1": "MODIS 全球植被指数与地表反射率 (MOD13Q1)",
  "science.aria.s1-gunw.v1": "NASA ARIA Sentinel-1 标准解缠干涉图 (S1-GUNW)",
  "atmospheric.sentinel-5p.l2.v1": "Sentinel-5P 大气污染与温室气体监测 (TROPOMI)",
  "science.gee.catalog.v1": "Google Earth Engine (GEE) 全球公共数据目录",
  "science.gee.s2-sr.v1": "GEE: Sentinel-2 MSI 地表反射率 (S2_SR_HARMONIZED)",
  "science.gee.s1-grd.v1": "GEE: Sentinel-1 C 波段 SAR GRD (S1_GRD)",
  "science.gee.landsat9.v1": "GEE: Landsat 9 C2-L2 地表反射率 (LC09_L2)",
  "science.gee.landsat8.v1": "GEE: Landsat 8 C2-L2 地表反射率 (LC08_L2)",
  "science.gee.dynamic-world.v1": "GEE: Dynamic World 10m 实时全球土地覆盖",
  "science.gee.worldcover.v1": "GEE: ESA WorldCover 全球 10 米土地利用",
  "science.gee.gsw.v1": "GEE: JRC 全球地表水体动态分布 (GSW)",
  "science.gee.dem-glo30.v1": "GEE: Copernicus DEM GLO-30 全球 30m 高程",
  "science.gee.nasadem.v1": "GEE: NASA NASADEM 全球 30m 高程",
  "science.gee.gem-15.v1": "GEE: Google GEM-15 15m 高程",
  "science.gee.modis-ndvi.v1": "GEE: MODIS 全球 250m 植被指数 (MOD13Q1)",
  "science.gee.era5-land.v1": "GEE: ECMWF ERA5-Land 逐小时气象再分析",
  "science.gee.s5p-no2.v1": "GEE: Sentinel-5P 大气 NO2 对流层柱浓度",
  "science.whu.clcd.v1": "武汉大学中国年度 30m 土地覆盖 (CLCD v1.0.4)",
  "science.tum.eot20.v1": "TUM EOT20 全球经验海洋潮汐模型",
  "science.gebco.bathymetry-2026.v1": "GEBCO 全球 15 弧秒海底地形网格",
  "science.noaa.etopo-2022.v1": "NOAA ETOPO 2022 全球无缝地形水深模型",
  "science.nasa.black-marble.v1": "NASA Black Marble 全球夜光 (VIIRS)",
  "science.nasa.emit.l2a.v1": "NASA EMIT 高光谱地表反射率与矿物",
  "science.dlr.enmap.l2a.v1": "DLR EnMAP 高光谱 L2A 地表反射率",
  "science.nasa.ecostress.l2t.v1": "NASA ECOSTRESS 地表温度与蒸散发",
  "science.aws.sentinel-1-grd.v1": "Sentinel-1 GRD (AWS Open Data)",
  "science.noaa.aws-weather.v1": "NOAA GFS / GOES 气象卫星 (AWS Open Data)",
  "science.overture.maps.v1": "Overture Maps 全球开源矢量 (AWS Open Data)",
};

/**
 * 将后端 resource_id 映射为规范、简洁的人类可读数据源名称
 */
export function canonicalSourceDisplayName(resourceId: string): string {
  const trimmed = resourceId.trim();
  if (!trimmed) return "";
  if (CANONICAL_SHORT_NAMES[trimmed]) {
    return CANONICAL_SHORT_NAMES[trimmed];
  }
  const meta = CATALOG_METADATA_REGISTRY[trimmed];
  if (meta?.name) {
    return meta.name;
  }
  return trimmed;
}

/**
 * Returns the localized human-readable data source name for `locale`, preserving
 * `canonicalSourceDisplayName` for `"zh-CN"`.
 */
export function localizedSourceDisplayName(resourceId: string, locale: Locale): string {
  const trimmed = resourceId.trim();
  if (!trimmed) return "";
  if (locale !== "zh-CN") {
    const meta = CATALOG_METADATA_REGISTRY[trimmed];
    if (meta) {
      const localized = applyCatalogItemLocale(meta, trimmed, locale);
      if (localized?.name) return localized.name;
    }
    const translated = CATALOG_ITEM_TRANSLATIONS[trimmed]?.[locale]?.name;
    if (translated) return translated;
  }
  return canonicalSourceDisplayName(trimmed);
}

/**
 * 读取最近使用的数据源 ID 列表（fail-closed）
 */
export function getRecentSourceIds(): string[] {
  try {
    const storage = getStorage();
    if (!storage) return [];
    const raw = storage.getItem(RECENT_CATALOG_SOURCES_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return parsed
        .filter(
          (id): id is string =>
            typeof id === "string" &&
            id.trim().length > 0 &&
            !RETIRED_OR_LOCAL_RESOURCE_IDS.has(id.trim())
        )
        .slice(0, MAX_RECENT_SOURCES);
    }
  } catch {
    // Fail-closed to empty array
  }
  return [];
}

/**
 * 记录最近使用的数据源（置顶并限制最大条数）
 */
export function recordRecentSourceId(resourceId: string): void {
  if (!resourceId || typeof resourceId !== "string") return;
  const trimmed = resourceId.trim();
  if (!trimmed || RETIRED_OR_LOCAL_RESOURCE_IDS.has(trimmed)) return;
  try {
    const current = getRecentSourceIds();
    const updated = [trimmed, ...current.filter((id) => id !== trimmed)].slice(0, MAX_RECENT_SOURCES);
    const storage = getStorage();
    storage?.setItem(RECENT_CATALOG_SOURCES_STORAGE_KEY, JSON.stringify(updated));
  } catch {
    // Fail-closed safely
  }
}

/**
 * 清除最近使用数据源（用于测试与重置）
 */
export function clearRecentSources(): void {
  try {
    const storage = getStorage();
    storage?.removeItem(RECENT_CATALOG_SOURCES_STORAGE_KEY);
  } catch {
    // Fail-closed safely
  }
}

const failedVerificationAccounts = new Set<string>();

export function setAccountVerificationFailed(accountId: string, failed: boolean): void {
  const key = accountId.trim().toLowerCase();
  if (!key) return;
  if (failed) {
    failedVerificationAccounts.add(key);
  } else {
    failedVerificationAccounts.delete(key);
  }
}

export function isAccountVerificationFailed(accountId: string): boolean {
  return failedVerificationAccounts.has(accountId.trim().toLowerCase());
}

function hasOwnAccountStatus(
  accountStatuses: Record<string, string> | undefined,
  accountId: string
): boolean {
  return Boolean(
    accountStatuses &&
      Object.prototype.hasOwnProperty.call(accountStatuses, accountId)
  );
}

function isAccountConfiguredState(state: string | undefined, accountId?: string): boolean {
  if (accountId && isAccountVerificationFailed(accountId)) {
    return false;
  }
  return (
    state === "logged_in" ||
    state === "login_required" ||
    state === "not_required"
  );
}

/**
 * 综合判定单个数据源的展示状态
 */
export function resolveItemStatus(
  resourceId: string,
  meta: CatalogItemMetadata,
  descriptor?: SourceDescriptor,
  accountStatuses?: Record<string, string>
): CatalogItemStatus {
  if (meta.defaultStatus === "in_progress") {
    return "in_progress";
  }
  if (descriptor && descriptor.availability === "unavailable") {
    return "in_progress";
  }
  if (isSourceSearchOnly(resourceId)) {
    return "search_only";
  }
  if (descriptor && descriptor.availability === "degraded") {
    return "search_only";
  }

  if (resourceId.startsWith("science.gee.") && hasOwnAccountStatus(accountStatuses, "gee")) {
    return isAccountConfiguredState(accountStatuses?.gee, "gee")
      ? "available"
      : "search_only";
  }

  if (meta.accountRequirement === "account_required") {
    const requiredIds =
      meta.requiredAccountIds && meta.requiredAccountIds.length > 0
        ? meta.requiredAccountIds
        : (descriptor?.accounts ?? []).map((id) => String(id));
    if (
      accountStatuses &&
      requiredIds.length > 0 &&
      requiredIds.every((accId) => hasOwnAccountStatus(accountStatuses, accId))
    ) {
      const allReady = requiredIds.every((accId) =>
        isAccountConfiguredState(accountStatuses[accId], accId)
      );
      return allReady ? "available" : "account_required";
    }
    if (requiredIds.some((accId) => isAccountVerificationFailed(accId))) {
      return "account_required";
    }
  }

  if (descriptor && descriptor.accounts && descriptor.accounts.length > 0) {
    if (meta.accountRequirement === "account_required" && meta.defaultStatus === "account_required") {
      return "account_required";
    }
  }
  return meta.defaultStatus;
}

/**
 * 构造所有目录项（将静态元数据与动态 availableSources 融合）
 */
export function buildCatalogItems(
  availableSources: SourceDescriptor[] = [],
  accountStatuses?: Record<string, string>,
  locale: Locale = getCurrentLocale(),
): CatalogItem[] {
  const descriptorMap = new Map<string, SourceDescriptor>();
  for (const desc of availableSources) {
    descriptorMap.set(desc.resource_id, desc);
  }

  const items: CatalogItem[] = [];

  // 1. 遍历注册的元数据（已移除下架数据源、本地转换项与合并至家族条目的子级别）
  for (const [resourceId, rawMeta] of Object.entries(CATALOG_METADATA_REGISTRY)) {
    if (
      RETIRED_OR_LOCAL_RESOURCE_IDS.has(resourceId) ||
      COLLAPSED_FAMILY_SUBLEVEL_IDS.has(resourceId)
    ) {
      continue;
    }
    const meta = applyCatalogItemLocale(rawMeta, resourceId, locale);
    const desc = descriptorMap.get(resourceId);
    if (desc?.availability === "unavailable") {
      continue;
    }
    const status = resolveItemStatus(resourceId, meta, desc, accountStatuses);
    const disabled = status === "in_progress";

    let accountText = meta.accountText
      .replace(/（已配置）|\(configured\)/gi, "")
      .trim();
    if (resourceId.startsWith("science.gee.") && hasOwnAccountStatus(accountStatuses, "gee")) {
      accountText = localizeCatalogDynamicAccountText(
        locale,
        status === "available" ? "gee_ok" : "gee_search",
      );
    } else if (meta.accountRequirement === "account_required") {
      const baseLabel = meta.accountText.replace(/（.*?）/g, "").replace(/\(.*?\)/g, "").trim();
      const requiredIds = meta.requiredAccountIds ?? [];
      const hasLiveRequired =
        accountStatuses !== undefined &&
        requiredIds.length > 0 &&
        requiredIds.every((id) => hasOwnAccountStatus(accountStatuses, id));
      const hasFailedVerify = requiredIds.some(
        (id) => isAccountVerificationFailed(id) || accountStatuses?.[id] === "auth_failed"
      );
      if (hasLiveRequired || hasFailedVerify) {
        accountText =
          status === "available"
            ? baseLabel
            : hasFailedVerify
              ? localizeCatalogDynamicAccountText(locale, "failed", baseLabel)
              : localizeCatalogDynamicAccountText(locale, "missing", baseLabel);
      }
    }

    const normalizedAccountRequirement: "free" | "account_required" =
      meta.accountRequirement === "account_required" ? "account_required" : "free";

    items.push({
      ...meta,
      accountRequirement: normalizedAccountRequirement,
      name: meta.name || desc?.display_name_zh || resourceId,
      accountText,
      resourceId,
      status,
      disabled,
    });
  }

  // 2. 容错处理：若 availableSources 中存在未在静态注册表里的在线资源，也补充展示（排除已下架、合并子级别与本地 DEM）
  for (const desc of availableSources) {
    if (
      RETIRED_OR_LOCAL_RESOURCE_IDS.has(desc.resource_id) ||
      COLLAPSED_FAMILY_SUBLEVEL_IDS.has(desc.resource_id) ||
      desc.availability === "unavailable"
    ) {
      continue;
    }
    if (!CATALOG_METADATA_REGISTRY[desc.resource_id]) {
      const family = desc.family;
      let category: CatalogCategoryKey = "primary-sar";
      let group: "orbiting" | "historical" | "dem" | "auxiliary" = "orbiting";
      const subcategories: CatalogCategoryKey[] = ["primary-all"];

      if (family === "optical") {
        category = "primary-optical";
        subcategories.push("primary-optical");
      } else if (family === "dem") {
        category = "primary-dem";
        group = "dem";
        subcategories.push("primary-dem");
      } else {
        subcategories.push("primary-sar");
      }

      const requiredIds = (desc.accounts ?? []).map((id) => String(id));
      const needsAccount = requiredIds.length > 0;
      const hasLiveRequired =
        accountStatuses !== undefined &&
        requiredIds.length > 0 &&
        requiredIds.every((accId) => hasOwnAccountStatus(accountStatuses, accId));
      const isAccountReady =
        !needsAccount ||
        !hasLiveRequired ||
        requiredIds.every((accId) => isAccountConfiguredState(accountStatuses?.[accId], accId));

      const status: CatalogItemStatus =
        isSourceSearchOnly(desc.resource_id) || desc.availability === "degraded"
          ? "search_only"
          : isAccountReady
            ? "available"
            : "account_required";

      items.push({
        resourceId: desc.resource_id,
        name: desc.display_name_zh || desc.resource_id,
        subtitle: `${desc.group_id} · ${family}`,
        band: family.toUpperCase(),
        group,
        category,
        subcategories,
        defaultStatus: status,
        status,
        disabled: false,
        description: desc.display_name_zh || desc.resource_id,
        accountRequirement: needsAccount ? "account_required" : "free",
        requiredAccountIds: requiredIds,
        accountText: needsAccount ? desc.accounts.join(" / ") : localizeCatalogDynamicAccountText(locale, "free"),
        filters: [locale === "en" ? "Spatial Extent" : locale === "ja" ? "空間範囲" : "空间范围"],
      });
    }
  }

  return items;
}

export interface FilterCatalogOptions {
  category: CatalogCategoryKey;
  ignoreCategory?: boolean;
  query?: string;
  band?: CatalogBandTag;
  account?: CatalogAccountTag;
  recentIds?: string[];
}

/**
 * 纯函数：根据分类、关键词和标签过滤目录条目
 */
export function filterCatalogItems(
  items: CatalogItem[],
  options: FilterCatalogOptions
): CatalogItem[] {
  const {
    category,
    ignoreCategory = false,
    query = "",
    band = "all",
    account = "all",
    recentIds = [],
  } = options;

  const normalizedQuery = query.trim().toLowerCase();

  return items.filter((item) => {
    // 1. 分类匹配
    if (!ignoreCategory) {
      if (category === "recent") {
        if (!recentIds.includes(item.resourceId)) {
          return false;
        }
      } else if (category === "primary-all") {
        if (!item.subcategories.includes("primary-all")) return false;
      } else if (category === "auxiliary-all") {
        if (!item.subcategories.includes("auxiliary-all")) return false;
      } else {
        if (item.category !== category && !item.subcategories.includes(category)) {
          return false;
        }
      }
    }

    // 2. 波段标签过滤
    if (band !== "all") {
      if (band === "optical") {
        if (item.bandTag !== "optical") return false;
      } else {
        if (item.bandTag !== band) return false;
      }
    }

    // 3. 账号标签过滤
    if (account !== "all") {
      if (account === "free" && item.accountRequirement !== "free") return false;
      if (account === "account_required" && item.accountRequirement !== "account_required") return false;
    }

    // 4. 关键词搜索匹配（名称、ID、说明、波段、平台、卫星）
    if (normalizedQuery) {
      const matchName = item.name.toLowerCase().includes(normalizedQuery);
      const matchId = item.resourceId.toLowerCase().includes(normalizedQuery);
      const matchSubtitle = item.subtitle.toLowerCase().includes(normalizedQuery);
      const matchBand = (item.band || "").toLowerCase().includes(normalizedQuery);
      const matchSatellites = (item.satellites || "").toLowerCase().includes(normalizedQuery);
      const matchDesc = item.description.toLowerCase().includes(normalizedQuery);
      const matchNotes = (item.usageNotes || []).some((n) => n.toLowerCase().includes(normalizedQuery));
      if (!matchName && !matchId && !matchSubtitle && !matchBand && !matchSatellites && !matchDesc && !matchNotes) {
        return false;
      }
    }

    return true;
  });
}

/**
 * 统计各分类包含的条目总数（不受中栏关键词影响，反映分类整体规模）
 */
export function computeCategoryCounts(
  items: CatalogItem[],
  recentIds: string[]
): Record<CatalogCategoryKey, number> {
  const counts: Record<CatalogCategoryKey, number> = {
    recent: 0,
    "primary-all": 0,
    "primary-sar": 0,
    "primary-optical": 0,
    "primary-dem": 0,
    "primary-thematic": 0,
    "auxiliary-all": 0,
    "auxiliary-orbit": 0,
    "auxiliary-atmospheric": 0,
    "auxiliary-geodesy": 0,
    "gee-catalog": 0,
  };

  const registeredIds = new Set(items.map((i) => i.resourceId));
  counts.recent = recentIds.filter((id) => registeredIds.has(id)).length;

  for (const item of items) {
    for (const sub of item.subcategories) {
      if (sub in counts) {
        counts[sub] += 1;
      }
    }
  }

  return counts;
}

/**
 * 键盘导航纯逻辑：计算下一个可选（非 disabled）条目的索引
 */
export function getNextSelectableIndex(
  items: CatalogItem[],
  currentIndex: number,
  direction: "up" | "down"
): number {
  if (items.length === 0) return -1;
  const selectableIndices = items
    .map((item, idx) => (item.disabled ? -1 : idx))
    .filter((idx) => idx !== -1);

  if (selectableIndices.length === 0) return -1;

  if (currentIndex === -1 || !selectableIndices.includes(currentIndex)) {
    return selectableIndices[0];
  }

  const currentSelectablePos = selectableIndices.indexOf(currentIndex);
  if (direction === "down") {
    const nextPos = (currentSelectablePos + 1) % selectableIndices.length;
    return selectableIndices[nextPos];
  } else {
    const prevPos = (currentSelectablePos - 1 + selectableIndices.length) % selectableIndices.length;
    return selectableIndices[prevPos];
  }
}
