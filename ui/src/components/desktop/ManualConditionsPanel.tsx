import React, { useState, useMemo, useRef, useEffect } from "react";
import {
  RotateCcw,
  ChevronDown,
  ChevronRight,
  Upload,
  Search,
  PanelLeft,
  PanelLeftOpen,
  Orbit,
  Loader2,
  FolderOpen,
  FileCode,
  X,
  Layers,
  AlertTriangle,
  Square,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  WORKBENCH_LOCALE_TEXT,
  XYZ_COMPRESSION_LOCALE,
  XYZ_EXPORT_FORMAT_LOCALE,
  XYZ_PRESET_BASEMAPS_LOCALE,
} from "@/lib/manualWorkbenchLocale";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import { modShortcut } from "@/lib/platformShortcuts";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import type { AoiPreviewOk, Bbox } from "@/lib/bridge";
import type { SourceDescriptor, SourceView } from "@/shell/api/types";
import {
  initialGroupState,
  toggleGroup,
  sourceSummary,
  timeSummary,
  lastSearchSummary,
  quickRange,
  validateConditions,
  groupOrbitTitle,
  type ManualConditionsGroup,
  type QuickRangeKind,
} from "@/shell/api/manualConditionsLogic";
import {
  sourceOptions,
  conditionsKind,
} from "@/shell/api/sourceConditions";
import {
  SEARCH_FILTER_SCHEMA_CATALOG,
  facetFilters,
  draftSelection,
  draftText,
  localizeSearchFilterLabel,
  localizeSearchFilterPlaceholder,
  localizeSearchFilterOptionLabel,
  type SearchFilterDraft,
} from "@/lib/searchFilterSchema";
import {
  estimateXyzTilesForBbox,
  formatTileCount,
  formatXyzByteSize,
  getXyzSourceMaxZoom,
  normalizeXyzZooms,
} from "@/lib/xyzTileEstimator";
import type { GeodataRasterOverlaySpec } from "@/components/WorkbenchMap";
import {
  type WaybackPeriodSelection,
  isValidWaybackVersionId,
  scaleWaybackEstimateForPeriods,
} from "@/lib/waybackClient";
import { WaybackPanel } from "./WaybackPanel";
import {
  GEE_CATALOG_THEMES,
  GEE_PUBLIC_CATALOG_INDEX,
  filterGeePublicCatalog,
  isStaticGeeAssetId,
  lookupGeePublicDataset,
  type GeeCatalogTheme,
} from "@/lib/geePublicCatalogIndex";
import {
  AGENT_VECTOR_EXTENSIONS,
  isAcceptedVectorFileName,
  vectorAoiDisplayName,
} from "@/shell/api/agentComposerLogic";
import {
  previewAoiFileBytes,
  setWorkingAoiGeojsonFeatures,
  pickDirectory,
  pickOpenFile,
  planDemDownloadBbox,
  planLocalDemConversion,
  queryOpenGeodata,
} from "@/lib/bridge";
import { explainActionError } from "@/shell/api/errorExplanations";
import { bboxArrayToBbox, readFileAsBase64 } from "@/shell/api/vectorAoiImport";
import { importVectorFromHtml5Files } from "@/lib/vectorDropImport";
import { DEM_DATASET_OPTIONS } from "@/lib/demSources";
import {
  sarFilterProfile,
  resolveEffectiveNumericBounds,
  allowedPolarizationsForBeamMode,
  visibleFilterFields,
  NISAR_OFFICIAL_PRODUCT_LEVELS,
  isNisarLogicalResourceId,
  isNisarSBandResourceId,
  resolveNisarProductLevel,
} from "@/shell/api/sarFilterProfile";
import { AdminRegionSearch } from "./AdminRegionSearch";
import { VectorFeaturePickerModal } from "./VectorFeaturePickerModal";
import { geometryAreaKm2 } from "@/lib/geometryArea";
import {
  CATALOG_METADATA_REGISTRY,
  canonicalSourceDisplayName,
  localizedSourceDisplayName,
} from "@/lib/dataSourceCatalogLogic";
import { applyCatalogItemLocale, localizeGeeDatasetEntry, localizeGeeThemeLabel } from "@/lib/dataSourceCatalogTranslations";
import { parseGacosManualDatesList } from "@/lib/companionGacosFlow";
import { SpSelect } from "@/components/ui/SpSelect";
import { GeeWorkbenchPanel, type GeeSyncedResultsPayload } from "./GeeCatalogAutocomplete";

export interface WorkbenchConvertSummary {
  ok: boolean;
  completed: boolean;
  sourceDatum: string;
  targetDatum: string;
  geoidModel: string | null;
  requiresConversion: boolean;
  message: string;
  rawDemPath: string;
  ellipsoidDemPath: string;
  sarscapeDemPath: string;
  outputDir: string;
}

export interface WorkbenchDemPlanSummary {
  ok: boolean;
  dataset: string;
  tileCount: number;
  sourceDatum: string;
  targetDatum: string;
  rawDemPath: string;
  sarscapeDemPath: string;
  outputDir: string;
}

export const DERIVED_SAR_INTERFEROGRAM_IDS: ReadonlySet<string> = new Set([
  "science.aria.s1-gunw.v1",
  "science.opera.disp-s1.v1",
  "science.comet.licsar.v1",
]);

/**
 * 判断数据源是否为衍生干涉成品（ARIA / OPERA / LiCSAR），此类产品已预配准或完成干涉处理，
 * 在工作台顶栏应归类为「雷达 SAR」且不应显示原始 Sentinel-1 SLC 精密轨道 (POEORB 20 天) 警告。
 */
export function isDerivedSarInterferogram(resourceId?: string | null): boolean {
  if (!resourceId) return false;
  return DERIVED_SAR_INTERFEROGRAM_IDS.has(resourceId.trim());
}

export interface WorkbenchToolOptionsSnapshot {
  convertSourceDatum: "auto" | "EGM2008" | "EGM96" | "WGS84_ELLIPSOID";
  convertOutputMode: "sarscape" | "ellipsoid";
  convertOutputDir: string;
  demOutputDir: string;
  orbitOutputDir: string;
  orbitActiveSubMode?: "current" | "local";
  gacosUtcTime?: string;
  gacosDatesText: string;
  gacosUtcHour: string;
  gacosUtcMinute: string;
  gacosOutputFormat: "geotiff" | "binary";
  xyzTileSource: string;
  xyzTileCustomUrl: string;
  xyzTileCoordSys: "auto" | "wgs84" | "gcj02";
  xyzTileZoom: number;
  xyzTileZooms?: number[];
  xyzWaybackVersionId: string;
  xyzWaybackSelectedPeriods: WaybackPeriodSelection[];
  xyzWaybackProbedMaxZoom: number | null;
  xyzTileStitchMosaic: boolean;
  xyzTileExportFormat: "gtiff" | "cog" | "mbtiles" | "zip" | "png" | "jpg" | "envi" | "vrt";
  xyzTileCompression: "DEFLATE" | "LZW" | "JPEG" | "WEBP" | "NONE";
  xyzTileBuildPyramids: boolean;
  xyzTileOutputDir: string;
  openGeodataSubProduct: string;
  openGeodataOutputDir: string;
  nisarProductLevel?: string;
  nisarReleaseTiers?: ("validated" | "provisional")[];
  geeAssetId: string;
  geeTheme?: GeeCatalogTheme;
  geeSelectedPresetId?: string;
  geeMaxCloud?: number;
  geeSelectedBand?: string;
  geeSarPolarization?: string;
  geeTemporalStat?: string;
  selectedProviders: Record<string, string>;
  lastConvertSummary: WorkbenchConvertSummary | null;
  lastDemPlanSummary: WorkbenchDemPlanSummary | null;
}

export interface MultiProviderOption {
  id: string;
  label: string;
  subtitle: string;
  badge?: string;
  defaultGeeAssetId?: string;
}

export interface MultiProviderConfig {
  resourceId: string;
  category: "sentinel-2" | "landsat" | "dem";
  title: string;
  options: MultiProviderOption[];
}

export const MULTI_PROVIDER_CONFIGS: Record<string, MultiProviderConfig> = {
  "science.sentinel-2.l2a.v1": {
    resourceId: "science.sentinel-2.l2a.v1",
    category: "sentinel-2",
    title: "数据来源与格式",
    options: [
      {
        id: "cdse",
        label: "ESA CDSE (官方原始 SAFE 包)",
        subtitle: "欧空局官方原档 · 适于几何校正与科研 · 需 CDSE 账号",
      },
      {
        id: "mpc",
        label: "Planetary Computer / Earth Search (COG)",
        subtitle: "微软行星计算机与 AWS Earth Search 云优化单波段 COG · 免账号",
      },
      {
        id: "gee",
        label: "Google Earth Engine (裁剪 GeoTIFF)",
        subtitle: "云原生按 AOI 裁剪 · COPERNICUS/S2_SR_HARMONIZED",
        defaultGeeAssetId: "COPERNICUS/S2_SR_HARMONIZED",
      },
    ],
  },
  "science.sentinel-2.l1c.v1": {
    resourceId: "science.sentinel-2.l1c.v1",
    category: "sentinel-2",
    title: "数据来源与格式",
    options: [
      {
        id: "cdse",
        label: "ESA CDSE (官方原始 SAFE 包)",
        subtitle: "欧空局官方公开目录检索 · 下载需 CDSE 账号",
      },
      {
        id: "gee",
        label: "Google Earth Engine (裁剪 GeoTIFF)",
        subtitle: "云原生按 AOI 裁剪 · COPERNICUS/S2_HARMONIZED",
        defaultGeeAssetId: "COPERNICUS/S2_HARMONIZED",
      },
    ],
  },
  "science.landsat.c2-l2.v1": {
    resourceId: "science.landsat.c2-l2.v1",
    category: "landsat",
    title: "数据来源与格式",
    options: [
      {
        id: "mpc",
        label: "Planetary Computer (COG)",
        subtitle: "微软行星计算机单波段 COG 展开集 · 免账号",
      },
      {
        id: "usgs",
        label: "USGS M2M (官方原始 .tar)",
        subtitle: "公开目录统一检索 · 下载交付 USGS 官方完整 .tar（需 USGS 账号）",
      },
      {
        id: "gee",
        label: "Google Earth Engine (裁剪 GeoTIFF)",
        subtitle: "云端按 AOI 裁剪地表反射率 · LANDSAT/LC09/C02/T1_L2",
        defaultGeeAssetId: "LANDSAT/LC09/C02/T1_L2",
      },
    ],
  },
  "dem.cop30.v1": {
    resourceId: "dem.cop30.v1",
    category: "dem",
    title: "数据来源与格式",
    options: [
      {
        id: "aws",
        label: "AWS Open Data (GLO-30)",
        subtitle: "亚马逊公开数据 GLO-30 原始瓦片 · 免账号",
      },
      {
        id: "opentopography",
        label: "OpenTopography",
        subtitle: "按范围聚合 GeoTIFF · 需 API Key",
      },
      {
        id: "gee",
        label: "Google Earth Engine (高程裁剪)",
        subtitle: "云端按 AOI 裁剪高程 · COPERNICUS/DEM/GLO30",
        defaultGeeAssetId: "COPERNICUS/DEM/GLO30",
      },
    ],
  },
  "COP30": {
    resourceId: "COP30",
    category: "dem",
    title: "数据来源与格式",
    options: [
      {
        id: "aws",
        label: "AWS Open Data (GLO-30)",
        subtitle: "亚马逊公开数据 GLO-30 原始瓦片 · 免账号",
      },
      {
        id: "opentopography",
        label: "OpenTopography",
        subtitle: "按范围聚合 GeoTIFF · 需 API Key",
      },
      {
        id: "gee",
        label: "Google Earth Engine (高程裁剪)",
        subtitle: "云端按 AOI 裁剪高程 · COPERNICUS/DEM/GLO30",
        defaultGeeAssetId: "COPERNICUS/DEM/GLO30",
      },
    ],
  },
};

export function localizeMultiProviderConfig(
  cfg: MultiProviderConfig,
  locale: keyof typeof WORKBENCH_LOCALE_TEXT,
): MultiProviderConfig {
  if (locale === "zh-CN") return cfg;
  const sd = WORKBENCH_LOCALE_TEXT[locale].sourceDelivery;
  return {
    ...cfg,
    title: sd.title,
    options: cfg.options.map((opt) => {
      if (cfg.resourceId === "science.sentinel-2.l2a.v1") {
        if (opt.id === "cdse") return { ...opt, label: sd.s2CdseLabel, subtitle: sd.s2CdseSub };
        if (opt.id === "mpc") return { ...opt, subtitle: sd.s2CogSub };
        if (opt.id === "gee") return { ...opt, label: sd.s2GeeLabel, subtitle: sd.s2GeeSub };
      } else if (cfg.resourceId === "science.sentinel-2.l1c.v1") {
        if (opt.id === "cdse") return { ...opt, label: sd.s2L1cCdseLabel, subtitle: sd.s2L1cCdseSub };
        if (opt.id === "gee") return { ...opt, label: sd.s2GeeLabel, subtitle: sd.s2L1cGeeSub };
      } else if (cfg.resourceId === "science.landsat.c2-l2.v1") {
        if (opt.id === "mpc") return { ...opt, subtitle: sd.l89MpcSub };
        if (opt.id === "usgs") return { ...opt, label: sd.l89UsgsLabel, subtitle: sd.l89UsgsSub };
        if (opt.id === "gee") return { ...opt, label: sd.l89GeeLabel, subtitle: sd.l89GeeSub };
      } else if (cfg.resourceId === "dem.cop30.v1" || cfg.resourceId === "COP30") {
        if (opt.id === "aws") return { ...opt, subtitle: sd.demAwsSub };
        if (opt.id === "opentopography") return { ...opt, subtitle: sd.demOtSub };
        if (opt.id === "gee") return { ...opt, label: sd.demGeeLabel, subtitle: sd.demGeeSub };
      }
      return opt;
    }),
  };
}

export function isMultiProviderDataset(resourceId: string): boolean {
  return Boolean(MULTI_PROVIDER_CONFIGS[resourceId]);
}

export function getActiveDatasetProvider(resourceId: string): string {
  const cfg = MULTI_PROVIDER_CONFIGS[resourceId];
  if (!cfg) return "default";
  const state = getWorkbenchToolOptions().selectedProviders;
  const current = state[resourceId];
  if (current && cfg.options.some((o) => o.id === current)) {
    return current;
  }
  return cfg.options[0].id;
}

export function setActiveDatasetProvider(resourceId: string, providerId: string): void {
  const cfg = MULTI_PROVIDER_CONFIGS[resourceId];
  if (!cfg) return;
  const matchedOpt = cfg.options.find((o) => o.id === providerId);
  const current = getWorkbenchToolOptions().selectedProviders;
  const nextPatch: Partial<WorkbenchToolOptionsSnapshot> = {
    selectedProviders: {
      ...current,
      [resourceId]: providerId,
    },
  };
  if (matchedOpt?.defaultGeeAssetId) {
    nextPatch.geeAssetId = matchedOpt.defaultGeeAssetId;
  }
  updateWorkbenchToolOptions(nextPatch);
}

let workbenchToolOptionsState: WorkbenchToolOptionsSnapshot = {
  convertSourceDatum: "auto",
  convertOutputMode: "sarscape",
  convertOutputDir: "",
  demOutputDir: "",
  orbitOutputDir: "",
  orbitActiveSubMode: "current",
  gacosUtcTime: "10:00",
  gacosDatesText: "",
  gacosUtcHour: "10",
  gacosUtcMinute: "00",
  gacosOutputFormat: "geotiff",
  xyzTileSource: "arcgis_satellite",
  xyzTileCustomUrl: "",
  xyzTileCoordSys: "auto",
  xyzTileZoom: 15,
  xyzTileZooms: [15],
  xyzWaybackVersionId: "",
  xyzWaybackSelectedPeriods: [],
  xyzWaybackProbedMaxZoom: null,
  xyzTileStitchMosaic: true,
  xyzTileExportFormat: "gtiff",
  xyzTileCompression: "DEFLATE",
  xyzTileBuildPyramids: true,
  xyzTileOutputDir: "",
  openGeodataSubProduct: "",
  openGeodataOutputDir: "",
  nisarProductLevel: "RSLC",
  nisarReleaseTiers: ["validated", "provisional"],
  geeAssetId: "ALL",
  geeTheme: "all",
  geeSelectedPresetId: "science.gee.catalog.v1",
  geeMaxCloud: 30,
  geeSelectedBand: "",
  geeSarPolarization: "ALL",
  geeTemporalStat: "median",
  selectedProviders: {
    "science.sentinel-2.l2a.v1": "cdse",
    "science.sentinel-2.l1c.v1": "cdse",
    "science.landsat.c2-l2.v1": "mpc",
    "dem.cop30.v1": "aws",
    "COP30": "aws",
  },
  lastConvertSummary: null,
  lastDemPlanSummary: null,
};

const workbenchToolOptionsListeners = new Set<() => void>();

export function getWorkbenchToolOptions(): WorkbenchToolOptionsSnapshot {
  return workbenchToolOptionsState;
}

export function updateWorkbenchToolOptions(
  patch: Partial<WorkbenchToolOptionsSnapshot>
): WorkbenchToolOptionsSnapshot {
  workbenchToolOptionsState = { ...workbenchToolOptionsState, ...patch };
  for (const listener of workbenchToolOptionsListeners) {
    try {
      listener();
    } catch {}
  }
  return workbenchToolOptionsState;
}

export function subscribeWorkbenchToolOptions(listener: () => void): () => void {
  workbenchToolOptionsListeners.add(listener);
  return () => {
    workbenchToolOptionsListeners.delete(listener);
  };
}

export interface GacosWorkbenchInputAudit {
  spanStatus: "missing" | "too_small" | "too_large" | "ok";
  lonSpanDeg: number;
  latSpanDeg: number;
  validDates: string[];
  invalidTokens: string[];
  batchCount: number;
  utcHourNum: number;
  utcMinuteNum: number;
}

export function inspectGacosWorkbenchInputs(
  bbox: Bbox | null | undefined,
  datesText: string,
  hourStr: string,
  minuteStr: string
): GacosWorkbenchInputAudit {
  let spanStatus: GacosWorkbenchInputAudit["spanStatus"] = "missing";
  let lonSpanDeg = 0;
  let latSpanDeg = 0;
  if (
    bbox &&
    Number.isFinite(bbox.west) &&
    Number.isFinite(bbox.east) &&
    Number.isFinite(bbox.south) &&
    Number.isFinite(bbox.north)
  ) {
    lonSpanDeg = Math.abs(bbox.east - bbox.west);
    latSpanDeg = Math.abs(bbox.north - bbox.south);
    if (lonSpanDeg > 10.0 || latSpanDeg > 10.0) {
      spanStatus = "too_large";
    } else if (lonSpanDeg < 0.05 || latSpanDeg < 0.05) {
      spanStatus = "too_small";
    } else {
      spanStatus = "ok";
    }
  }
  const { validDates, invalidTokens } = parseGacosManualDatesList(datesText || "");
  const h = Number.parseInt(hourStr, 10);
  const m = Number.parseInt(minuteStr, 10);
  return {
    spanStatus,
    lonSpanDeg,
    latSpanDeg,
    validDates,
    invalidTokens,
    batchCount: validDates.length > 0 ? Math.ceil(validDates.length / 20) : 0,
    utcHourNum: Number.isInteger(h) && h >= 0 && h <= 23 ? h : 10,
    utcMinuteNum: Number.isInteger(m) && m >= 0 && m <= 59 ? m : 0,
  };
}

export function parentDirFromPath(filePath: string): string {
  const trimmed = (filePath || "").trim();
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return idx > 0 ? trimmed.slice(0, idx) : "";
}

export function resolveEffectiveDemSourceDatum(
  inputPath: string,
  requested: "auto" | "EGM2008" | "EGM96" | "WGS84_ELLIPSOID"
): { datum: "EGM2008" | "EGM96" | "WGS84_ELLIPSOID"; inferredByFallback: boolean } {
  if (requested !== "auto") {
    return { datum: requested, inferredByFallback: false };
  }
  const lower = (inputPath || "").toLowerCase();
  if (lower.includes("ellipsoid") || lower.includes("wgs84")) {
    return { datum: "WGS84_ELLIPSOID", inferredByFallback: false };
  }
  if (/egm96|nasadem|srtm|aw3d30/.test(lower)) {
    return { datum: "EGM96", inferredByFallback: false };
  }
  if (/egm2008|cop30|cop90|copernicus|glo[-_ ]?30|glo[-_ ]?90/.test(lower)) {
    return { datum: "EGM2008", inferredByFallback: false };
  }
  return { datum: "EGM2008", inferredByFallback: true };
}

function shortPlatformBadgeLabel(plat: string): string {
  const match = /^Sentinel-1([A-D])$/i.exec(plat.trim());
  if (match) return `S1${match[1].toUpperCase()}`;
  return plat;
}

const DEM_DATASET_SPECS: Record<
  string,
  { resolution: string; datum: string; geoid: string; channel: string }
> = {
  COP30: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM2008 正高",
    geoid: "EGM2008",
    channel: "AWS 公开桶 · 免账号直连",
  },
  COP90: {
    resolution: "90m (3 arc-sec)",
    datum: "EGM2008 正高",
    geoid: "EGM2008",
    channel: "AWS 公开桶 · 免账号直连",
  },
  SRTM_GL1: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM96 正高",
    geoid: "EGM96",
    channel: "OpenTopography · 需 API Key",
  },
  SRTM_GL3: {
    resolution: "90m (3 arc-sec)",
    datum: "EGM96 正高",
    geoid: "EGM96",
    channel: "OpenTopography · 需 API Key",
  },
  NASADEM: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM96 正高",
    geoid: "EGM96",
    channel: "OpenTopography · 需 API Key",
  },
  AW3D30: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM96 正高",
    geoid: "EGM96",
    channel: "OpenTopography · 需 API Key",
  },
  SRTM_GL1_ELLIPSOIDAL: {
    resolution: "30m (1 arc-sec)",
    datum: "WGS84 椭球高",
    geoid: "WGS84",
    channel: "OpenTopography · 需 API Key",
  },
  AW3D30_ELLIPSOIDAL: {
    resolution: "30m (1 arc-sec)",
    datum: "WGS84 椭球高",
    geoid: "WGS84",
    channel: "OpenTopography · 需 API Key",
  },
  ASTER_GDEM_V3: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM96 正高",
    geoid: "EGM96",
    channel: "NASA LP DAAC · 需 Earthdata 账号",
  },
  FABDEM: {
    resolution: "30m (1 arc-sec)",
    datum: "EGM2008 裸地正高",
    geoid: "EGM2008",
    channel: "公开 COG（CC BY-NC-SA 4.0 非商用）",
  },
  GEM15_DTM: {
    resolution: "15m (0.5 arc-sec)",
    datum: "EGM96 裸地正高",
    geoid: "EGM96",
    channel: "Google Cloud 公开 COG（CC BY 4.0 · 不含中国）",
  },
  GEM15_DSM: {
    resolution: "15m (0.5 arc-sec)",
    datum: "EGM96 地表正高",
    geoid: "EGM96",
    channel: "Google Cloud 公开 COG（CC BY 4.0 · 不含中国）",
  },
  NISAR_DEM: {
    resolution: "30m (1 arc-sec)",
    datum: "WGS84 椭球高",
    geoid: "WGS84",
    channel: "ASF Earthdata · 需账号",
  },
};

export interface ManualConditionsPanelProps {
  collapsed?: boolean;
  onCollapsedChange?: (collapsed: boolean) => void;
  expandedColumnCount?: number;
  width?: number;
  focusCondition?: { group: ManualConditionsGroup; field?: string; requestId: string } | null;
  conditionsChanged?: boolean;
  scientificMode: any;
  onScientificModeChange: (mode: any) => void;
  onResetParams: () => void;
  availableSources: SourceDescriptor[];
  activeResourceId: string;
  onSelectSource: (resourceId: string) => void;
  onOpenCatalog?: (opts?: { geeOnly?: boolean }) => void;
  platforms?: string[];
  onPlatformsChange?: (platforms: string[]) => void;
  productType: string;
  onProductTypeChange: (productType: string) => void;
  polarizations?: string[];
  onPolarizationsChange?: (polarizations: string[]) => void;
  beamModes?: string[];
  onBeamModesChange?: (beamModes: string[]) => void;
  lookDirection?: string | null;
  onLookDirectionChange?: (lookDirection: string | null) => void;
  opticalDraft?: SearchFilterDraft;
  onOpticalDraftChange?: (draft: SearchFilterDraft) => void;
  workingAoi: Bbox | null;
  workingGeometry: Record<string, unknown> | null;
  aoiLabel: string;
  onReplaceAoi: (geometry: Record<string, unknown> | null, label: string, bbox: [number, number, number, number] | null) => Promise<void>;
  onPreviewAdminGeometry: (geometry: Record<string, unknown> | null) => void;
  onImportVectorSuccess: (geometry: Record<string, unknown>, bbox: Bbox, aoiLabel: string) => void;
  startDate: string | null;
  endDate: string | null;
  onStartDateChange: (startDate: string) => void;
  onEndDateChange: (endDate: string) => void;
  orbitDirection: any;
  onOrbitDirectionChange: (dir: any) => void;
  relativeOrbit: string | number | null;
  onRelativeOrbitChange: (ro: string) => void;
  path?: string | number | null;
  onPathChange?: (path: string) => void;
  frame: string | number | null;
  onFrameChange: (f: string) => void;
  searching: boolean;
  searchError?: string | null;
  onTriggerSearch: () => void;
  onCancelSearch?: () => void;
  cancellingSearch?: boolean;
  activeSourceView?: SourceView | null;

  // 工具模式相关属性 (A3)
  demSource?: string;
  onDemSourceChange?: (source: string) => void;
  autoConvertEllipsoid?: boolean;
  onToggleAutoConvertEllipsoid?: () => void;
  orbitSourceType?: "current" | "local";
  onOrbitSourceTypeChange?: (type: "current" | "local") => void;
  localSlcPath?: string;
  onChangeLocalSlcPath?: (path: string) => void;
  onPickLocalSlcPath?: () => void;
  localDemPath?: string;
  onChangeLocalDemPath?: (path: string) => void;
  onPickLocalDemPath?: () => void;
  inlineToolResult?: { ok: boolean; message: string } | null;
  onDismissToolResult?: () => void;
  // 数据 / 工具分段切换 (改版第 8 步)
  activeNavTab?: "data" | "tools";
  onNavTabChange?: (tab: "data" | "tools") => void;
  activeToolId?: string;
  onRunTool?: (toolId: string, params: Record<string, any>) => Promise<void> | void;
  /** Suppress inline multi-provider radio group when DesktopWorkbench renders the top authoritative bar. */
  hideMultiProviderSelector?: boolean;
  onSyncGeeResults?: (payload: GeeSyncedResultsPayload) => void;
  onPreviewWaybackOverlay?: (overlay: GeodataRasterOverlaySpec | null) => void;
}

function GeeDatasetSelectorSection({
  assetId,
  theme,
  onChangeTheme,
  onChangeAssetId,
}: {
  assetId: string;
  theme: GeeCatalogTheme;
  onChangeTheme: (nextTheme: GeeCatalogTheme) => void;
  onChangeAssetId: (nextAssetId: string) => void;
}) {
  const { locale } = useI18n();
  const G = WORKBENCH_LOCALE_TEXT[locale].geeInline;
  const matches = useMemo(
    () => filterGeePublicCatalog(theme, "").map((d) => localizeGeeDatasetEntry(d, locale)),
    [theme, locale]
  );
  const currentMeta = useMemo(() => {
    const found =
      assetId && assetId !== "ALL"
        ? GEE_PUBLIC_CATALOG_INDEX.find((d) => d.assetId === assetId) ?? null
        : null;
    return found ? localizeGeeDatasetEntry(found, locale) : null;
  }, [assetId, locale]);

  const selectOptions = useMemo(() => {
    const allLabel =
      theme === "all"
        ? G.allCatalogOption.replace("{total}", String(GEE_PUBLIC_CATALOG_INDEX.length))
        : G.themeCatalogOption.replace("{count}", String(matches.length));
    const base = [
      { value: "ALL", label: allLabel },
      ...matches.map((d) => ({
        value: d.assetId,
        label: `${d.title} (${d.resolution}) · ${d.assetId}`,
      })),
    ];
    if (assetId && assetId !== "ALL" && !base.some((o) => o.value === assetId)) {
      base.splice(1, 0, {
        value: assetId,
        label: currentMeta
          ? `${currentMeta.title} (${currentMeta.resolution}) · ${assetId}`
          : G.customAssetSuffix.replace("{id}", assetId),
      });
    }
    return base;
  }, [matches, assetId, currentMeta, theme, G]);

  return (
    <div className="flex flex-col gap-2 pt-1" data-testid="gee-catalog-dataset-picker">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs text-sp-text-secondary">
          {G.publicCatalogHeading
            .replace("{shown}", String(matches.length))
            .replace("{total}", String(GEE_PUBLIC_CATALOG_INDEX.length))}
        </span>
      </div>
      <div className="flex flex-wrap gap-1">
        {GEE_CATALOG_THEMES.map((th) => {
          const active = theme === th.id;
          return (
            <button
              key={th.id}
              type="button"
              onClick={() => onChangeTheme(th.id)}
              className={cn(
                "px-2 py-0.5 rounded-full text-xs border transition-colors cursor-pointer whitespace-nowrap",
                active
                  ? "bg-sp-selected border-sp-text text-sp-text font-medium"
                  : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover"
              )}
            >
              {localizeGeeThemeLabel(th.id, th.label, locale)}
            </button>
          );
        })}
      </div>
      <SpSelect
        value={assetId || "ALL"}
        onChange={onChangeAssetId}
        options={selectOptions}
        aria-label={G.selectAssetAria}
      />
      <details data-testid="gee-custom-id-disclosure" className="group">
        <summary className="text-xs text-sp-text-muted hover:text-sp-text cursor-pointer select-none">
          {G.customAssetLabel}
        </summary>
        <div className="flex flex-col gap-1 mt-1.5">
          <input
            type="text"
            placeholder={G.customAssetPlaceholder}
            value={assetId === "ALL" ? "" : assetId}
            onChange={(e) => onChangeAssetId(e.target.value.trim() || "ALL")}
            className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1 focus:ring-sp-accent"
          />
          <span className="text-xs text-sp-text-muted leading-snug">
            {G.customAssetNote}
          </span>
        </div>
      </details>
    </div>
  );
}

export function ManualConditionsPanel({
  collapsed: controlledCollapsed,
  onCollapsedChange,
  expandedColumnCount = 3,
  width = 320,
  focusCondition = null,
  conditionsChanged = false,
  scientificMode,
  onScientificModeChange,
  onResetParams,
  availableSources,
  activeResourceId,
  onSelectSource,
  onOpenCatalog,
  platforms,
  onPlatformsChange,
  productType,
  onProductTypeChange,
  polarizations,
  onPolarizationsChange,
  beamModes = [],
  onBeamModesChange,
  lookDirection = null,
  onLookDirectionChange,
  opticalDraft,
  onOpticalDraftChange,
  workingAoi,
  workingGeometry,
  aoiLabel,
  onReplaceAoi,
  onPreviewAdminGeometry,
  onImportVectorSuccess,
  startDate,
  endDate,
  onStartDateChange,
  onEndDateChange,
  orbitDirection,
  onOrbitDirectionChange,
  relativeOrbit,
  onRelativeOrbitChange,
  path,
  onPathChange,
  frame,
  onFrameChange,
  searching,
  searchError,
  onTriggerSearch,
  onCancelSearch,
  cancellingSearch = false,
  activeSourceView,
  demSource = "COP30",
  onDemSourceChange,
  autoConvertEllipsoid = true,
  onToggleAutoConvertEllipsoid,
  orbitSourceType = "current",
  onOrbitSourceTypeChange,
  localSlcPath = "",
  onChangeLocalSlcPath,
  onPickLocalSlcPath,
  localDemPath = "",
  onChangeLocalDemPath,
  onPickLocalDemPath,
  inlineToolResult,
  onDismissToolResult,
  activeNavTab,
  onNavTabChange,
  activeToolId,
  onRunTool,
  hideMultiProviderSelector = false,
  onSyncGeeResults,
  onPreviewWaybackOverlay,
}: ManualConditionsPanelProps) {
  const { t, locale } = useI18n();
  const [localCollapsed, setLocalCollapsed] = useState<boolean>(false);
  const isStrip = controlledCollapsed ?? localCollapsed;
  const [internalNavTab, setInternalNavTab] = useState<"data" | "tools">("data");
  const [pluginModalOpen, setPluginModalOpen] = useState<boolean>(false);
  const [gacosDatePickerValue, setGacosDatePickerValue] = useState<string>("");
  const [gacosSlcDir, setGacosSlcDir] = useState<string>("");
  const [gacosScanBusy, setGacosScanBusy] = useState<boolean>(false);
  const [gacosScanNote, setGacosScanNote] = useState<{ ok: boolean; text: string } | null>(null);
  const currentNavTab = activeNavTab ?? internalNavTab;
  const handleNavTabSelect = (tab: "data" | "tools") => {
    setInternalNavTab(tab);
    onNavTabChange?.(tab);
  };
  const setIsStrip = (collapsed: boolean) => {
    setLocalCollapsed(collapsed);
    onCollapsedChange?.(collapsed);
  };
  const groupRefs = useRef<Partial<Record<ManualConditionsGroup, HTMLDivElement | null>>>({});
  const expandForFocusRef = useRef(onCollapsedChange);
  expandForFocusRef.current = onCollapsedChange;
  const focusedRequestRef = useRef<string | null>(null);
  React.useEffect(() => {
    if (!focusCondition || focusedRequestRef.current === focusCondition.requestId) return;
    focusedRequestRef.current = focusCondition.requestId;
    setLocalCollapsed(false);
    expandForFocusRef.current?.(false);
    setGroupState((current) => ({ ...current, [focusCondition.group]: true }));
    const timer = window.setTimeout(() => {
      const group = groupRefs.current[focusCondition.group];
      group?.scrollIntoView?.({ block: "nearest", behavior: "auto" });
      group?.querySelector<HTMLElement>("[aria-expanded]")?.focus();
      group?.setAttribute("data-condition-focused", "true");
    }, 0);
    return () => window.clearTimeout(timer);
  }, [focusCondition]);
  const [groupState, setGroupState] = useState<Record<ManualConditionsGroup, boolean>>(
    initialGroupState()
  );
  const [vectorBusy, setVectorBusy] = useState<boolean>(false);
  const showVectorSpinner = useDelayedFlag(vectorBusy);
  const showSearchSpinner = useDelayedFlag(searching);
  const [searchElapsedMs, setSearchElapsedMs] = useState(0);
  React.useEffect(() => {
    if (!searching) {
      setSearchElapsedMs(0);
      return;
    }
    const startedAt = Date.now();
    setSearchElapsedMs(0);
    const timer = window.setInterval(() => {
      setSearchElapsedMs(Date.now() - startedAt);
    }, 100);
    return () => window.clearInterval(timer);
  }, [searching]);
  const searchElapsedSeconds = searchElapsedMs / 1000;
  const showSearchElapsed = searching && searchElapsedSeconds >= 1;
  const [vectorError, setVectorError] = useState<string | null>(null);
  const [pendingVectorPreview, setPendingVectorPreview] = useState<AoiPreviewOk | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [toolOptions, setToolOptions] = useState<WorkbenchToolOptionsSnapshot>(() =>
    getWorkbenchToolOptions()
  );
  const [toolPreviewBusy, setToolPreviewBusy] = useState<boolean>(false);

  const isWaybackSource =
    activeResourceId === "auxiliary.xyz-tiles.v1" && toolOptions.xyzTileSource === "esri_wayback";
  const xyzStaticMaxZoom = getXyzSourceMaxZoom(toolOptions.xyzTileSource);
  const xyzSourceMaxZoom =
    isWaybackSource &&
    typeof toolOptions.xyzWaybackProbedMaxZoom === "number" &&
    Number.isFinite(toolOptions.xyzWaybackProbedMaxZoom)
      ? Math.min(xyzStaticMaxZoom, Math.max(1, Math.round(toolOptions.xyzWaybackProbedMaxZoom)))
      : xyzStaticMaxZoom;
  const waybackPeriodCount = useMemo(() => {
    if (!isWaybackSource) return 1;
    if (toolOptions.xyzWaybackSelectedPeriods.length > 0) {
      return toolOptions.xyzWaybackSelectedPeriods.length;
    }
    return isValidWaybackVersionId(toolOptions.xyzWaybackVersionId) ? 1 : 0;
  }, [isWaybackSource, toolOptions.xyzWaybackSelectedPeriods.length, toolOptions.xyzWaybackVersionId]);
  const selectedXyzZooms = useMemo(() => {
    const rawZooms = (opticalDraft as (SearchFilterDraft & { xyz_zooms?: unknown }) | undefined)?.xyz_zooms;
    if (Array.isArray(rawZooms)) {
      return Array.from(
        new Set(
          rawZooms
            .map((z) => Math.round(Number(z)))
            .filter((z) => Number.isFinite(z) && z >= 1 && z <= xyzSourceMaxZoom)
        )
      ).sort((a, b) => a - b);
    }
    const rawSingle = Number((opticalDraft as Record<string, unknown> | undefined)?.xyz_zoom);
    if (Number.isFinite(rawSingle) && rawSingle >= 1 && rawSingle <= xyzSourceMaxZoom) {
      return [Math.round(rawSingle)];
    }
    if (Array.isArray(toolOptions.xyzTileZooms)) {
      return Array.from(
        new Set(
          toolOptions.xyzTileZooms
            .map((z) => Math.round(Number(z)))
            .filter((z) => Number.isFinite(z) && z >= 1 && z <= xyzSourceMaxZoom)
        )
      ).sort((a, b) => a - b);
    }
    const fallback = Math.min(xyzSourceMaxZoom, Math.max(1, Math.round(toolOptions.xyzTileZoom || 15)));
    return [fallback];
  }, [opticalDraft, toolOptions.xyzTileZooms, toolOptions.xyzTileZoom, xyzSourceMaxZoom]);

  const setXyzZooms = (nextZooms: number[]) => {
    const sorted = Array.from(
      new Set(
        nextZooms
          .map((z) => Math.round(Number(z)))
          .filter((z) => Number.isFinite(z) && z >= 1 && z <= xyzSourceMaxZoom)
      )
    ).sort((a, b) => a - b);
    const primaryZoom = sorted[0] ?? Math.min(15, xyzSourceMaxZoom);
    updateWorkbenchToolOptions({
      xyzTileZooms: sorted,
      xyzTileZoom: primaryZoom,
    });
    if (onOpticalDraftChange) {
      onOpticalDraftChange({
        ...(opticalDraft ?? {}),
        xyz_zooms: sorted as unknown as SearchFilterDraft[string],
        xyz_zoom: String(primaryZoom),
      });
    }
  };

  const xyzEstimate = useMemo(() => {
    if (activeResourceId !== "auxiliary.xyz-tiles.v1" || !workingAoi || selectedXyzZooms.length === 0) {
      return null;
    }
    return estimateXyzTilesForBbox(workingAoi, selectedXyzZooms, {
      source: toolOptions.xyzTileSource,
      exportFormat: toolOptions.xyzTileExportFormat,
      stitchMosaic: toolOptions.xyzTileStitchMosaic,
      compression: toolOptions.xyzTileCompression,
      includeOverviews: toolOptions.xyzTileBuildPyramids,
      maxZoomOverride: isWaybackSource ? toolOptions.xyzWaybackProbedMaxZoom : undefined,
    });
  }, [
    activeResourceId,
    isWaybackSource,
    workingAoi,
    selectedXyzZooms,
    toolOptions.xyzTileSource,
    toolOptions.xyzWaybackProbedMaxZoom,
    toolOptions.xyzTileStitchMosaic,
    toolOptions.xyzTileExportFormat,
    toolOptions.xyzTileCompression,
    toolOptions.xyzTileBuildPyramids,
  ]);

  const wasWaybackSourceRef = useRef<boolean>(false);
  const lastClippedDraftRef = useRef<{ draft: unknown; ceiling: number } | null>(null);

  // Clip selected zooms and reset stale Wayback preview/probe state cleanly on source switches (Conditions F2 & F3).
  useEffect(() => {
    if (!isWaybackSource) {
      if (toolOptions.xyzWaybackProbedMaxZoom !== null) {
        updateWorkbenchToolOptions({ xyzWaybackProbedMaxZoom: null });
      }
      if (wasWaybackSourceRef.current) {
        wasWaybackSourceRef.current = false;
        onPreviewWaybackOverlay?.(null);
      }
      lastClippedDraftRef.current = null;
      return;
    }
    wasWaybackSourceRef.current = true;
    const rawOptsZooms = Array.isArray(toolOptions.xyzTileZooms)
      ? toolOptions.xyzTileZooms
      : [toolOptions.xyzTileZoom];
    const hasOutOfRangeOpts =
      rawOptsZooms.some((z) => z > xyzSourceMaxZoom) || toolOptions.xyzTileZoom > xyzSourceMaxZoom;
    if (hasOutOfRangeOpts) {
      const clippedOpts = normalizeXyzZooms(rawOptsZooms, Math.min(15, xyzSourceMaxZoom), xyzSourceMaxZoom);
      const nextPrimary = clippedOpts[clippedOpts.length - 1] ?? Math.min(15, xyzSourceMaxZoom);
      updateWorkbenchToolOptions({
        xyzTileZooms: clippedOpts,
        xyzTileZoom: nextPrimary,
      });
    }

    const draftZoomsRaw = (opticalDraft as (SearchFilterDraft & { xyz_zooms?: unknown }) | undefined)?.xyz_zooms;
    const draftSingleRaw = Number((opticalDraft as Record<string, unknown> | undefined)?.xyz_zoom);
    const hasOutOfRangeDraft =
      (Array.isArray(draftZoomsRaw) && draftZoomsRaw.some((z) => Number(z) > xyzSourceMaxZoom)) ||
      (Number.isFinite(draftSingleRaw) && draftSingleRaw > xyzSourceMaxZoom);
    const alreadyHandledDraft =
      lastClippedDraftRef.current?.draft === opticalDraft &&
      lastClippedDraftRef.current?.ceiling === xyzSourceMaxZoom;
    if (hasOutOfRangeDraft && !alreadyHandledDraft && onOpticalDraftChange) {
      lastClippedDraftRef.current = { draft: opticalDraft, ceiling: xyzSourceMaxZoom };
      const draftSource = Array.isArray(draftZoomsRaw)
        ? draftZoomsRaw.map((z) => Number(z))
        : rawOptsZooms;
      const clippedDraft = normalizeXyzZooms(draftSource, Math.min(15, xyzSourceMaxZoom), xyzSourceMaxZoom);
      const nextDraftPrimary = clippedDraft[clippedDraft.length - 1] ?? Math.min(15, xyzSourceMaxZoom);
      onOpticalDraftChange({
        ...(opticalDraft ?? {}),
        xyz_zooms: clippedDraft as unknown as SearchFilterDraft[string],
        xyz_zoom: String(nextDraftPrimary),
      });
    }
  }, [
    isWaybackSource,
    xyzSourceMaxZoom,
    toolOptions.xyzWaybackProbedMaxZoom,
    toolOptions.xyzTileZooms,
    toolOptions.xyzTileZoom,
    opticalDraft,
    onOpticalDraftChange,
    onPreviewWaybackOverlay,
  ]);

  React.useEffect(() => {
    const sync = () => setToolOptions(getWorkbenchToolOptions());
    workbenchToolOptionsListeners.add(sync);
    return () => {
      workbenchToolOptionsListeners.delete(sync);
    };
  }, []);

  // 当选择新的本地 DEM 路径时，若未手动指定输出目录，则默认同步为该 DEM 文件所在父目录并预计算转换方案
  React.useEffect(() => {
    const trimmed = (localDemPath || "").trim();
    if (!trimmed) return;
    const parent = parentDirFromPath(trimmed);
    const current = getWorkbenchToolOptions();
    const resolved = resolveEffectiveDemSourceDatum(trimmed, current.convertSourceDatum);
    const effectiveOutputDir = current.convertOutputDir.trim() || parent;
    void planLocalDemConversion(trimmed, effectiveOutputDir, resolved.datum)
      .then((res) => {
        if (res && res.ok) {
          updateWorkbenchToolOptions({
            lastConvertSummary: {
              ok: true,
              completed: false,
              sourceDatum: res.auto?.source || resolved.datum,
              targetDatum: res.auto?.target || "WGS84_ELLIPSOID",
              geoidModel: res.auto?.geoid_model || (resolved.datum === "EGM96" ? "EGM96" : resolved.datum === "EGM2008" ? "EGM2008" : null),
              requiresConversion: Boolean(res.auto?.requires_conversion),
              message:
                (res.auto?.message || "") +
                (resolved.inferredByFallback ? WORKBENCH_LOCALE_TEXT[locale].convMode.datumFallbackNote : ""),
              rawDemPath: String(res.plan?.raw_dem_path || trimmed),
              ellipsoidDemPath: String(res.plan?.ellipsoid_dem_path || ""),
              sarscapeDemPath: String(res.plan?.sarscape_ready_dem_path || ""),
              outputDir: effectiveOutputDir,
            },
          });
        }
      })
      .catch(() => {});
  }, [localDemPath, toolOptions.convertSourceDatum, toolOptions.convertOutputMode, toolOptions.convertOutputDir, locale]);

  const estimatedDemTileCount = useMemo(() => {
    if (!workingAoi) return 0;
    const lonSpan = Math.max(1, Math.floor(workingAoi.east) - Math.floor(workingAoi.west) + 1);
    const latSpan = Math.max(1, Math.floor(workingAoi.north) - Math.floor(workingAoi.south) + 1);
    return lonSpan * latSpan;
  }, [workingAoi]);

  const handlePreviewDemPlan = async () => {
    if (!workingAoi || toolPreviewBusy) return;
    setToolPreviewBusy(true);
    try {
      const targetOut = toolOptions.demOutputDir.trim();
      const plan = await planDemDownloadBbox(
        workingAoi.west,
        workingAoi.east,
        workingAoi.south,
        workingAoi.north,
        targetOut,
        demSource
      );
      if (plan && plan.ok) {
        const p = (plan.plan || {}) as Record<string, unknown>;
        updateWorkbenchToolOptions({
          lastDemPlanSummary: {
            ok: true,
            dataset: String(p.dataset || demSource),
            tileCount: typeof p.tile_count === "number" ? p.tile_count : estimatedDemTileCount,
            sourceDatum: String(p.source_vertical_datum || (demSource.startsWith("COP") ? "EGM2008" : "EGM96")),
            targetDatum: autoConvertEllipsoid ? "WGS84_ELLIPSOID" : String(p.source_vertical_datum || "EGM2008"),
            rawDemPath: String(p.raw_dem_path || ""),
            sarscapeDemPath: autoConvertEllipsoid ? String(p.sarscape_ready_dem_path || "") : "",
            outputDir: targetOut,
          },
        });
      }
    } catch {
    } finally {
      setToolPreviewBusy(false);
    }
  };

  const handlePreviewLocalDemConversion = async () => {
    const trimmed = (localDemPath || "").trim();
    if (!trimmed || toolPreviewBusy) return;
    setToolPreviewBusy(true);
    try {
      const resolved = resolveEffectiveDemSourceDatum(trimmed, toolOptions.convertSourceDatum);
      const effectiveOut = toolOptions.convertOutputDir.trim() || parentDirFromPath(trimmed);
      const res = await planLocalDemConversion(trimmed, effectiveOut, resolved.datum);
      if (res && res.ok) {
        updateWorkbenchToolOptions({
          lastConvertSummary: {
            ok: true,
            completed: false,
            sourceDatum: res.auto?.source || resolved.datum,
            targetDatum: res.auto?.target || "WGS84_ELLIPSOID",
            geoidModel: res.auto?.geoid_model || (resolved.datum === "EGM96" ? "EGM96" : resolved.datum === "EGM2008" ? "EGM2008" : null),
            requiresConversion: Boolean(res.auto?.requires_conversion),
            message:
              (res.auto?.message || "") +
              (resolved.inferredByFallback ? WORKBENCH_LOCALE_TEXT[locale].convMode.datumFallbackNote : ""),
            rawDemPath: String(res.plan?.raw_dem_path || trimmed),
            ellipsoidDemPath: String(res.plan?.ellipsoid_dem_path || ""),
            sarscapeDemPath:
              toolOptions.convertOutputMode === "ellipsoid"
                ? ""
                : String(res.plan?.sarscape_ready_dem_path || ""),
            outputDir: effectiveOut,
          },
        });
      }
    } catch {
    } finally {
      setToolPreviewBusy(false);
    }
  };

  const conditionsScrollRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (scientificMode === "gee" && conditionsScrollRef.current) {
      conditionsScrollRef.current.scrollTop = 0;
    }
  }, [scientificMode, toolOptions.geeAssetId]);

  const profile = useMemo(() => sarFilterProfile(activeResourceId), [activeResourceId]);
  const effectiveRelativeOrbitProfile = useMemo(
    () => (profile?.relativeOrbit ? resolveEffectiveNumericBounds(profile.relativeOrbit, platforms) : null),
    [profile, platforms],
  );
  const effectivePathProfile = useMemo(
    () => (profile?.path ? resolveEffectiveNumericBounds(profile.path, platforms) : null),
    [profile, platforms],
  );
  const effectiveFrameProfile = useMemo(
    () =>
      profile?.frame
        ? {
            ...profile.frame,
            ...resolveEffectiveNumericBounds(profile.frame, platforms),
          }
        : null,
    [profile, platforms],
  );

  const validationErrors = useMemo(() => {
    if (scientificMode !== "sar") return [];
    return validateConditions({
      relativeOrbit,
      path,
      frame,
      relativeOrbitProfile: effectiveRelativeOrbitProfile,
      pathProfile: effectivePathProfile,
      frameProfile: effectiveFrameProfile,
      startDate,
      endDate,
    });
  }, [
    scientificMode,
    relativeOrbit,
    path,
    frame,
    effectiveRelativeOrbitProfile,
    effectivePathProfile,
    effectiveFrameProfile,
    startDate,
    endDate,
    locale,
  ]);

  const relativeOrbitError = validationErrors.find((e) => e.field === "relativeOrbit")?.message;
  const pathError = validationErrors.find((e) => e.field === "path")?.message;
  const frameError = validationErrors.find((e) => e.field === "frame")?.message;

  const handleToggle = (group: ManualConditionsGroup) => {
    setGroupState((prev) => toggleGroup(prev, group));
  };

  const sourceGroups = useMemo(() => sourceOptions(availableSources), [availableSources, locale]);
  const kind = useMemo(
    () => conditionsKind(activeResourceId, availableSources),
    [activeResourceId, availableSources]
  );
  const opticalSchema = useMemo(
    () => SEARCH_FILTER_SCHEMA_CATALOG.schemas.find((s) => s.resource_id === activeResourceId) || null,
    [activeResourceId]
  );
  const opticalFacets = useMemo(
    () => (opticalSchema ? facetFilters(opticalSchema) : []),
    [opticalSchema]
  );
  const opticalQualityFilters = useMemo(
    () => (opticalSchema ? opticalSchema.filters.filter((f) => f.group === "quality" || f.control === "number") : []),
    [opticalSchema]
  );

  const activeSourceDesc = useMemo(
    () => availableSources.find((s) => s.resource_id === activeResourceId) || null,
    [availableSources, activeResourceId]
  );

  const activeGeeAssetId =
    toolOptions.geeAssetId && toolOptions.geeAssetId !== "ALL"
      ? toolOptions.geeAssetId
      : CATALOG_METADATA_REGISTRY[activeResourceId]?.geeAssetId || "ALL";
  const isStaticGeeAssetSelected =
    activeResourceId.startsWith("science.gee.") && isStaticGeeAssetId(activeGeeAssetId);

  const aoiArea = geometryAreaKm2(workingGeometry);
  const aoiAreaText = aoiArea > 0
    ? `${aoiArea.toLocaleString(locale, { maximumFractionDigits: aoiArea < 100 ? 1 : 0 })} km²`
    : "";
  const timeInfo = useMemo(() => timeSummary(startDate, endDate), [startDate, endDate, locale]);
  const gacosAudit = useMemo(
    () =>
      inspectGacosWorkbenchInputs(
        workingAoi,
        toolOptions.gacosDatesText,
        toolOptions.gacosUtcHour,
        toolOptions.gacosUtcMinute
      ),
    [workingAoi, toolOptions.gacosDatesText, toolOptions.gacosUtcHour, toolOptions.gacosUtcMinute]
  );

  const lastSearchText = useMemo(() => {
    if (!activeSourceView?.searched_at) return "";
    return lastSearchSummary(activeSourceView.searched_at, activeSourceView.item_count, new Date());
  }, [activeSourceView, locale]);

  const handleQuick = (kind: QuickRangeKind) => {
    const range = quickRange(kind, new Date());
    onStartDateChange(range.startDate);
    onEndDateChange(range.endDate);
  };

  const finalizeVectorFeatures = async (
    preview: AoiPreviewOk,
    featureIds: string[],
    displayField: string,
    displayName: string
  ) => {
    if (!preview.geojson) {
      setVectorError(t("mapWorkbench.conditions.noGeometryFeatures"));
      return;
    }
    setVectorBusy(true);
    setVectorError(null);
    try {
      const result = await setWorkingAoiGeojsonFeatures(
        preview.geojson,
        featureIds,
        displayField,
        "merge",
        displayName
      );
      if (!result.ok) {
        setVectorError(explainActionError(result.error || t("mapWorkbench.conditions.vectorNormalizationFailed"), t("mapWorkbench.conditions.vectorImportFailed")));
        return;
      }
      const bbox = bboxArrayToBbox(result.bbox);
      if (!bbox || !result.geometry) {
        setVectorError(t("mapWorkbench.conditions.invalidVectorGeometry"));
        return;
      }
      setPendingVectorPreview(null);
      onImportVectorSuccess(
        result.geometry as Record<string, unknown>,
        bbox,
        result.display_name || displayName
      );
    } catch (err) {
      setVectorError(explainActionError(err, t("mapWorkbench.conditions.vectorImportFailed")));
    } finally {
      setVectorBusy(false);
    }
  };

  const processVectorFile = async (file: File) => {
    if (!isAcceptedVectorFileName(file.name)) {
      setVectorError(t("mapWorkbench.conditions.unsupportedVector"));
      return;
    }
    setVectorBusy(true);
    setVectorError(null);
    try {
      const base64Data = await readFileAsBase64(file);
      const preview = await previewAoiFileBytes(file.name, base64Data);
      if (!preview.ok) {
        setVectorError(explainActionError(preview.error || t("mapWorkbench.conditions.vectorPreviewFailed"), t("mapWorkbench.conditions.vectorImportFailed")));
        return;
      }
      if (!preview.geojson) {
        setVectorError(t("mapWorkbench.conditions.noGeometryFeatures"));
        return;
      }
      if (preview.features && preview.features.length > 1) {
        setPendingVectorPreview(preview);
        return;
      }
      const featureIds = preview.features
        ? preview.features.map(
            (f) => f.id || (f.properties?.id as string) || String(f.index ?? 0)
          )
        : [];
      const displayName = vectorAoiDisplayName(file.name) || t("mapWorkbench.conditions.vectorSelectionName");
      await finalizeVectorFeatures(preview, featureIds, preview.display_field || "", displayName);
    } catch (err) {
      setVectorError(explainActionError(err, t("mapWorkbench.conditions.vectorImportFailed")));
    } finally {
      setVectorBusy(false);
    }
  };

  const handleVectorFileSelect = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    e.target.value = "";
    await processVectorFile(file);
  };

  const handleAoiDrop = async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    if (files.length === 1 && !/\.shp$/i.test(files[0].name)) {
      await processVectorFile(files[0]);
      return;
    }
    setVectorBusy(true);
    setVectorError(null);
    try {
      const imported = await importVectorFromHtml5Files(files, { persistWorkingAoi: true });
      if (!imported.ok) {
        setVectorError(imported.error);
        return;
      }
      setPendingVectorPreview(null);
      onImportVectorSuccess(imported.geometry, imported.bbox, imported.label);
    } catch (err) {
      setVectorError(explainActionError(err, t("mapWorkbench.conditions.vectorImportFailed")));
    } finally {
      setVectorBusy(false);
    }
  };

  const effectiveOrbitSubMode: "current" | "local" =
    onOrbitSourceTypeChange ? orbitSourceType : (toolOptions.orbitActiveSubMode ?? orbitSourceType);
  const handleSelectOrbitSubMode = (mode: "current" | "local") => {
    updateWorkbenchToolOptions({ orbitActiveSubMode: mode });
    onOrbitSourceTypeChange?.(mode);
  };

  // 按钮文案跟随当前模式变化 (A3d + Item 8)
  const searchButtonLabel = useMemo(() => {
    const L = WORKBENCH_LOCALE_TEXT[locale];
    switch (scientificMode) {
      case "dem":
        return t("mapWorkbench.conditions.planDem");
      case "orbit":
        return effectiveOrbitSubMode === "local"
          ? L.orbitMode.localFolderBtn
          : t("mapWorkbench.conditions.matchOrbits");
      case "convert":
        return t("mapWorkbench.conditions.startConversion");
      case "sar":
      default:
        if (activeResourceId === "auxiliary.xyz-tiles.v1") {
          return isWaybackSource && waybackPeriodCount > 1
            ? L.xyz.downloadPeriodsBtn.replace("{count}", String(waybackPeriodCount))
            : L.xyz.downloadMosaicBtn;
        }
        if (
          activeResourceId === "orbit.sentinel-1.poeorb.v1" ||
          activeResourceId === "orbit.nisar.oe.v1" ||
          activeResourceId === "orbit.copernicus.pod.v1"
        ) {
          return activeResourceId === "orbit.sentinel-1.poeorb.v1" && effectiveOrbitSubMode === "local"
            ? L.orbitMode.localFolderBtn
            : locale === "en"
              ? "Search & Download Orbits"
              : locale === "ja"
                ? "精密軌道を検索・取得"
                : "检索并下载精密轨道";
        }
        if (activeResourceId === "atmospheric.gacos.ztd.v1") {
          return locale === "en"
            ? "Search / Fetch GACOS"
            : locale === "ja"
              ? "GACOS を検索・申請"
              : "查询并获取 GACOS 改正";
        }
        return t("mapWorkbench.conditions.search");
    }
  }, [scientificMode, activeResourceId, isWaybackSource, waybackPeriodCount, effectiveOrbitSubMode, locale, t]);

  // 按钮禁用校验
  const isSearchDisabled = useMemo(() => {
    if (searching) return true;
    if (scientificMode === "sar") {
      if (activeResourceId === "orbit.sentinel-1.poeorb.v1" && effectiveOrbitSubMode === "local") {
        return !localSlcPath;
      }
      if (activeResourceId === "auxiliary.xyz-tiles.v1") {
        if (
          !workingAoi ||
          selectedXyzZooms.length === 0 ||
          !xyzEstimate ||
          xyzEstimate.exceedsSourceMaxZoom ||
          xyzEstimate.exceedsHardLimit ||
          xyzEstimate.exceedsJpegDimensionLimit ||
          (isWaybackSource && waybackPeriodCount === 0)
        ) {
          return true;
        }
      }
      return validationErrors.length > 0;
    }
    if (scientificMode === "dem") return !workingAoi;
    if (scientificMode === "orbit") return effectiveOrbitSubMode === "local" && !localSlcPath;
    if (scientificMode === "convert") return !localDemPath;
    return false;
  }, [
    searching,
    scientificMode,
    activeResourceId,
    validationErrors.length,
    workingAoi,
    selectedXyzZooms.length,
    xyzEstimate,
    isWaybackSource,
    waybackPeriodCount,
    effectiveOrbitSubMode,
    localSlcPath,
    localDemPath,
  ]);

  // 收起后宽度为 0，不再保留竖条（由地图边缘浮动把手展开）
  if (isStrip) {
    return null;
  }

  const localizedActiveMeta = CATALOG_METADATA_REGISTRY[activeResourceId]
    ? applyCatalogItemLocale(CATALOG_METADATA_REGISTRY[activeResourceId], activeResourceId, locale)
    : undefined;
  const geePresetId = toolOptions.geeSelectedPresetId;
  const localizedPresetMeta =
    geePresetId && CATALOG_METADATA_REGISTRY[geePresetId]
      ? applyCatalogItemLocale(CATALOG_METADATA_REGISTRY[geePresetId], geePresetId, locale)
      : undefined;

  const activeSourceCardTitle =
    activeResourceId === "science.gee.catalog.v1" &&
    geePresetId &&
    geePresetId !== "science.gee.catalog.v1" &&
    CATALOG_METADATA_REGISTRY[geePresetId]?.geeAssetId === toolOptions.geeAssetId
      ? localizedPresetMeta?.name
      : localizedActiveMeta?.name ||
        (locale === "zh-CN"
          ? activeSourceDesc?.display_name_zh || canonicalSourceDisplayName(activeResourceId)
          : localizedSourceDisplayName(activeResourceId, locale));

  const activeSourceCardSubtitle =
    activeResourceId === "science.gee.catalog.v1"
      ? (!toolOptions.geeAssetId || toolOptions.geeAssetId === "ALL"
          ? WORKBENCH_LOCALE_TEXT[locale].geeInline.allCatalogSummarySuffix.replace(
              "{total}",
              String(GEE_PUBLIC_CATALOG_INDEX.length),
            )
          : `${toolOptions.geeAssetId} · ${
              localizedPresetMeta?.subtitle ||
              localizedActiveMeta?.subtitle ||
              sourceSummary(activeSourceDesc)
            }`)
      : (localizedActiveMeta?.subtitle ||
          sourceSummary(activeSourceDesc));

  const renderDataSourceCard = () => (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between">
        <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.source")}</span>
        <span className="text-xs text-sp-text-muted font-mono">{modShortcut("K")}</span>
      </div>

      <div
        role="button"
        tabIndex={0}
        data-testid="data-source-card"
        aria-label={t("panels.catalogModal.openCatalog")}
        onClick={() => onOpenCatalog?.()}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            onOpenCatalog?.();
          }
        }}
        className="eo-tactile-secondary p-2.5 rounded-[10px] border border-sp-line bg-sp-surface hover:bg-sp-hover cursor-pointer flex items-center justify-between gap-2 text-left shadow-xs group"
      >
        <div className="flex flex-col gap-0.5 min-w-0 flex-1">
          <span className="text-xs font-semibold text-sp-text truncate">
            {activeSourceCardTitle}
          </span>
          <span className="text-xs text-sp-text-muted truncate">
            {activeSourceCardSubtitle}
          </span>
        </div>
        <ChevronRight className="h-4 w-4 text-sp-text-muted group-hover:text-sp-text shrink-0" />
      </div>
    </div>
  );

  // 共享的 AOI 空间范围卡片（SAR 与 DEM 复用）
  const renderAoiGroup = () => (
    <div
      ref={(node) => { groupRefs.current.aoi = node; }}
      data-condition-group="aoi"
      data-condition-field="aoi"
      tabIndex={-1}
      onDragOver={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
      onDrop={handleAoiDrop}
      className="bg-sp-surface rounded-[10px] ring-1 ring-sp-line shrink-0 overflow-visible focus:outline-none"
    >
      <button
        type="button"
        aria-expanded={groupState.aoi}
        aria-controls="group-aoi-body"
        onClick={() => handleToggle("aoi")}
        className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
      >
        {groupState.aoi ? (
          <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
        ) : (
          <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
        )}
        <span>{t("mapWorkbench.conditions.spatialExtent")}</span>
      </button>
      {groupState.aoi && (
        <div id="group-aoi-body" className="p-3 pt-0 flex items-start gap-1 overflow-visible">
          <AdminRegionSearch
            workingAoi={workingAoi}
            workingGeometry={workingGeometry}
            aoiLabel={aoiLabel}
            areaText={aoiAreaText}
            onReplaceAoi={onReplaceAoi}
            onPreviewGeometry={onPreviewAdminGeometry}
          />
          <button
            type="button"
            disabled={vectorBusy}
            aria-busy={vectorBusy}
            onClick={() => fileInputRef.current?.click()}
            aria-label={t("mapWorkbench.conditions.importVector")}
            title={t("mapWorkbench.conditions.importVector")}
            className="h-9 w-[30px] shrink-0 rounded-[8px] border border-sp-line bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover disabled:opacity-50 inline-flex items-center justify-center transition-colors cursor-pointer"
          >
            {showVectorSpinner ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept={AGENT_VECTOR_EXTENSIONS.join(",")}
            onChange={handleVectorFileSelect}
            className="hidden"
          />
        </div>
      )}
      {vectorError && <span className="mt-1 block text-xs text-sp-danger-text leading-tight">{vectorError}</span>}
    </div>
  );

  const activeCatalogMeta = CATALOG_METADATA_REGISTRY[activeResourceId];
  const headerCategoryBadge = (() => {
    const B = WORKBENCH_LOCALE_TEXT[locale].categoryBadge;
    if (scientificMode === "gee") return "GEE";
    if (scientificMode === "dem") return "DEM";
    if (scientificMode === "orbit") {
      return B.orbit;
    }
    if (scientificMode === "convert") {
      return B.convert;
    }
    if (activeResourceId.startsWith("science.gee.")) return "GEE";
    // ARIA / OPERA / COMET-LiCSAR 均为衍生 SAR 干涉产品，绝不能落入光学或原始 SLC 分支
    if (isDerivedSarInterferogram(activeResourceId)) {
      return B.sar;
    }
    if (activeResourceId === "orbit.sentinel-1.poeorb.v1" || activeCatalogMeta?.category === "auxiliary-orbit") {
      return B.orbit;
    }
    if (activeCatalogMeta?.category === "auxiliary-atmospheric") {
      return B.atmo;
    }
    if (activeCatalogMeta?.category === "auxiliary-geodesy") {
      return B.geodesy;
    }
    if (activeCatalogMeta?.category === "primary-thematic") {
      return B.thematic;
    }
    if (
      (kind !== null && kind !== "sar") ||
      activeSourceDesc?.family === "optical" ||
      activeCatalogMeta?.category === "primary-optical" ||
      activeCatalogMeta?.bandTag === "optical"
    ) {
      return B.optical;
    }
    if (activeCatalogMeta?.group === "dem" || activeResourceId.startsWith("dem.")) {
      return "DEM";
    }
    if (activeCatalogMeta?.group === "auxiliary") {
      return B.aux;
    }
    return "SAR";
  })();

  const topModeSelectValue = (() => {
    if (scientificMode === "gee") return "gee";
    if (scientificMode === "convert") return "convert";
    return "sar";
  })();

  const handleTopModeSelectChange = (val: string) => {
    if (val === "gee") {
      onScientificModeChange("gee");
      return;
    }
    if (val === "convert" || val === "convert:sarscape" || val === "convert:ellipsoid") {
      onScientificModeChange("convert");
      return;
    }
    onScientificModeChange("sar");
  };

  return (
    <section
      aria-label={t("mapWorkbench.conditions.search")}
      data-column="search"
      data-collapsed="false"
      className="shrink-0 h-full flex flex-col bg-[#fafafa] dark:bg-[#141416] border-r border-sp-line select-none overflow-hidden"
      style={{ width }}
    >
      {/* 头部 42px (EOStage Compact Panel Header) */}
      <div className="h-[42px] shrink-0 flex items-center justify-between px-3 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416]">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-[13px] font-semibold text-sp-text tracking-tight whitespace-nowrap">
            {t("mapWorkbench.conditions.search")}
          </span>
          <span
            data-testid="conditions-header-category-badge"
            className="rounded-full bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text-muted whitespace-nowrap shrink-0"
          >
            {headerCategoryBadge}
          </span>
        </div>
        <div className="flex items-center gap-1 shrink-0">
          <button
            type="button"
            onClick={onResetParams}
            className="inline-flex items-center gap-1 text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover rounded-sp-sm px-2 py-1 transition-colors cursor-pointer whitespace-nowrap shrink-0"
          >
            <RotateCcw className="h-3.5 w-3.5 shrink-0" />
            <span>{t("mapWorkbench.conditions.reset")}</span>
          </button>
          <button
            type="button"
            onClick={() => { if (onCollapsedChange) onCollapsedChange(true); else setIsStrip(true); }}
            disabled={expandedColumnCount === 1}
            aria-label={t("mapWorkbench.conditions.collapseSearch")}
            title={expandedColumnCount === 1 ? t("mapWorkbench.conditions.keepOneColumn") : t("mapWorkbench.conditions.collapseSearch")}
            className="h-7 w-7 rounded-sp-sm inline-flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover disabled:opacity-40 disabled:cursor-not-allowed transition-colors cursor-pointer shrink-0"
          >
            <PanelLeft className="h-[14px] w-[14px] stroke-[1.75]" />
          </button>
        </div>
      </div>

      {/* 顶部模式切换：数据检索（含 SAR/光学/DEM/GACOS/精密轨道） + 纯处理工具 + GEE 云端计算与制图 */}
      <div className="px-3 pt-2 pb-1 shrink-0">
        <SpSelect
          data-testid="scientific-mode-select"
          size="md"
          aria-label={t("mapWorkbench.conditions.searchType")}
          value={topModeSelectValue}
          onChange={handleTopModeSelectChange}
          options={[
            {
              value: "sar",
              label: WORKBENCH_LOCALE_TEXT[locale].modes.search,
            },
            {
              value: "convert",
              label: WORKBENCH_LOCALE_TEXT[locale].modes.demConvert,
            },
            {
              value: "gee",
              label: WORKBENCH_LOCALE_TEXT[locale].modes.gee,
            },
          ]}
        />
      </div>

      {/* 条件组主体，每组保持自然高度 shrink-0，父级滚动 */}
      <div
        ref={conditionsScrollRef}
        data-conditions-scroll="true"
        className="flex-1 overflow-y-auto px-2.5 pt-2 pb-6 flex flex-col gap-2 min-h-0"
      >
        {/* ================= 模式 1: 影像检索 (SAR / 光学) ================= */}
        {scientificMode === "sar" && (
          <>
            {/* 1. 来源与卫星 */}
            <div
              ref={(node) => { groupRefs.current.source = node; }}
              data-condition-group="source"
              data-condition-field="source"
              tabIndex={-1}
              className="bg-sp-surface rounded-[10px] ring-1 ring-sp-line shrink-0 overflow-visible focus:outline-none"
            >
              <button
                type="button"
                aria-expanded={groupState.source}
                aria-controls="group-source-body"
                onClick={() => handleToggle("source")}
                className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
              >
                {groupState.source ? (
                  <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                )}
                <span className="shrink-0">{t("mapWorkbench.conditions.sourceAndSatellite")}</span>
                <span
                  className="ml-auto text-xs font-normal text-sp-text-muted truncate min-w-0"
                  title={activeSourceCardTitle}
                >
                  {activeSourceCardTitle}
                </span>
              </button>
              {groupState.source && (
                <div id="group-source-body" className="p-3 pt-0 flex flex-col gap-3">
                  <div>
                    {renderDataSourceCard()}

                    {/* 兼容无障碍和既有测试的 select */}
                    <select
                      value={activeResourceId}
                      onChange={(e) => {
                        void Promise.resolve(onSelectSource(e.target.value)).catch(() => {});
                      }}
                      aria-label={t("mapWorkbench.conditions.source")}
                      className="sr-only"
                      tabIndex={-1}
                    >
                      {sourceGroups.map((grp) => (
                        <optgroup key={grp.group} label={grp.group}>
                          {grp.options.map((opt) => (
                            <option
                              key={opt.value}
                              value={opt.value}
                              disabled={opt.disabled}
                              title={opt.title}
                            >
                              {opt.label}
                            </option>
                          ))}
                        </optgroup>
                      ))}
                    </select>
                  </div>

                  {/* Sentinel-2 处理级别快捷切换（L2A地表反射率 vs L1C 表观反射率，单行紧凑呈现） */}
                  {(activeResourceId === "science.sentinel-2.l2a.v1" ||
                    activeResourceId === "science.sentinel-2.l1c.v1") && (
                    <div className="flex flex-col gap-1 pt-0.5" data-testid="sentinel2-level-switcher">
                      <span className="text-xs text-sp-text-secondary font-medium">
                        {WORKBENCH_LOCALE_TEXT[locale].sar.processingLevel}
                      </span>
                      <div className="grid grid-cols-2 gap-1.5">
                        {[
                          {
                            id: "science.sentinel-2.l2a.v1",
                            title: WORKBENCH_LOCALE_TEXT[locale].sar.s2L2aTitle,
                          },
                          {
                            id: "science.sentinel-2.l1c.v1",
                            title: WORKBENCH_LOCALE_TEXT[locale].sar.s2L1cTitle,
                          },
                        ].map((lvl) => {
                          const isCurrent = activeResourceId === lvl.id;
                          return (
                            <button
                              key={lvl.id}
                              type="button"
                              data-testid={`s2-level-pill-${lvl.id === "science.sentinel-2.l2a.v1" ? "l2a" : "l1c"}`}
                              aria-pressed={isCurrent}
                              onClick={() => {
                                if (!isCurrent) {
                                  void Promise.resolve(onSelectSource(lvl.id)).catch(() => {});
                                }
                              }}
                              className={cn(
                                "h-8 px-2 rounded-sp-sm border text-center transition-colors cursor-pointer flex items-center justify-center min-w-0",
                                isCurrent
                                  ? "bg-sp-selected border-sp-text text-sp-text font-medium shadow-xs"
                                  : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                              )}
                            >
                              <span className="text-xs leading-tight whitespace-nowrap truncate">{lvl.title}</span>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  )}

                  {/* 多源提供商与交付模式分段选择器 (Strategy C) */}
                  {!hideMultiProviderSelector && isMultiProviderDataset(activeResourceId) && (() => {
                    const cfg = localizeMultiProviderConfig(
                      MULTI_PROVIDER_CONFIGS[activeResourceId],
                      locale,
                    );
                    const activeProvider = getActiveDatasetProvider(activeResourceId);
                    return (
                      <div className="flex flex-col gap-1.5 pt-1" data-testid="multi-provider-selector">
                        <span className="text-xs text-sp-text-secondary font-medium">
                          {cfg.title}
                        </span>
                        <div className="flex flex-col gap-1">
                          {cfg.options.map((opt) => {
                            const isSelected = activeProvider === opt.id;
                            return (
                              <button
                                key={opt.id}
                                type="button"
                                data-testid={`provider-option-${opt.id}`}
                                aria-checked={isSelected}
                                role="radio"
                                onClick={() => setActiveDatasetProvider(activeResourceId, opt.id)}
                                className={cn(
                                  "w-full px-2.5 py-2 rounded-sp-sm border text-left text-xs transition-colors flex flex-col items-start gap-0.5 cursor-pointer",
                                  isSelected
                                    ? "bg-sp-selected border-sp-text text-sp-text font-medium shadow-xs"
                                    : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                                )}
                              >
                                <span className="flex items-center gap-1.5 w-full min-w-0">
                                  <span
                                    className={cn(
                                      "inline-flex items-center justify-center w-3 h-3 rounded-full border shrink-0",
                                      isSelected
                                        ? "border-sp-text bg-sp-text"
                                        : "border-sp-text-muted bg-transparent"
                                    )}
                                  >
                                    {isSelected && <span className="w-1.5 h-1.5 rounded-full bg-sp-surface" />}
                                  </span>
                                  <span className="truncate">{opt.label}</span>
                                </span>
                                {opt.subtitle && (
                                  <span className="pl-[18px] text-xs leading-snug text-sp-text-muted break-words line-clamp-2">
                                    {opt.subtitle}
                                  </span>
                                )}
                              </button>
                            );
                          })}
                        </div>
                      </div>
                    );
                  })()}

                  {/* SAR 数据源字段：卫星平台 + 产品类型 */}
                  {kind === "sar" && (
                    <>
                      {profile?.platforms && profile.platforms.length > 1 && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.satellitePlatform")}</span>
                          <div
                            className={cn(
                              "pt-0.5",
                              profile.platforms.length === 4 &&
                                profile.platforms.every((p) => /^Sentinel-1[A-D]$/i.test(p))
                                ? "grid grid-cols-4 gap-1.5"
                                : "flex flex-wrap gap-1.5"
                            )}
                          >
                            {profile.platforms.map((plat) => {
                              const isSelected = (platforms || []).includes(plat);
                              const displayPlat = shortPlatformBadgeLabel(plat);
                              return (
                                <button
                                  key={plat}
                                  type="button"
                                  title={plat}
                                  aria-label={plat}
                                  onClick={() => {
                                    const cur = platforms || [];
                                    if (isSelected) {
                                      onPlatformsChange?.(cur.filter((p) => p !== plat));
                                    } else {
                                      onPlatformsChange?.([...cur, plat]);
                                    }
                                  }}
                                  className={cn(
                                    "h-[26px] px-2 rounded-sp-sm text-xs transition-colors cursor-pointer select-none inline-flex items-center justify-center border whitespace-nowrap",
                                    isSelected
                                      ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                      : "border-sp-line text-sp-text-secondary bg-sp-card hover:text-sp-text hover:bg-sp-hover"
                                  )}
                                >
                                  {displayPlat}
                                </button>
                              );
                            })}
                          </div>
                          {(platforms || []).includes("Sentinel-1B") &&
                            !(platforms || []).some((p) => p === "Sentinel-1A" || p === "Sentinel-1C" || p === "Sentinel-1D") && (
                              <span className="text-xs text-sp-warning-text leading-snug">
                                {WORKBENCH_LOCALE_TEXT[locale].sar.s1RetiredHint}
                              </span>
                            )}
                          {(platforms || []).includes("Sentinel-1A") &&
                            !(platforms || []).some((p) => p === "Sentinel-1C" || p === "Sentinel-1D") &&
                            (startDate || "") >= "2026-07-01" && (
                              <span className="text-xs text-sp-text-muted leading-snug">
                                {WORKBENCH_LOCALE_TEXT[locale].sar.s1RecentSwitchHint}
                              </span>
                            )}
                        </div>
                      )}

                      {profile?.productTypes && profile.productTypes.length > 0 && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.productType")}</span>
                          <div className="eo-segmented-track flex items-center w-full">
                            {[
                              { label: t("mapWorkbench.conditions.all"), val: "" },
                              ...profile.productTypes.map((t) => ({ label: t.label, val: t.label })),
                            ].map((type) => (
                              <button
                                key={type.label}
                                type="button"
                                data-active={(productType || "") === type.val}
                                onClick={() => onProductTypeChange(type.val)}
                                className="eo-segmented-pill flex-1 h-[24px] px-1 min-[1600px]:px-2 justify-center whitespace-nowrap cursor-pointer"
                              >
                                {type.label}
                              </button>
                            ))}
                          </div>
                        </div>
                      )}

                      {isNisarLogicalResourceId(activeResourceId) && (() => {
                        const S = WORKBENCH_LOCALE_TEXT[locale].sar;
                        const activeNisarLevel = resolveNisarProductLevel(
                          productType || toolOptions.nisarProductLevel
                        );
                        const isNisarSBand = isNisarSBandResourceId(activeResourceId);
                        const activeLevelMeta = NISAR_OFFICIAL_PRODUCT_LEVELS.find(
                          (item) => item.value === activeNisarLevel
                        );
                        const activeLevelDesc =
                          locale === "zh-CN"
                            ? (activeLevelMeta?.descriptionZh || activeNisarLevel)
                            : `${activeLevelMeta?.label || activeNisarLevel} (${activeLevelMeta?.scienceLevel || "L1"})`;
                        return (
                          <div className="flex flex-col gap-1.5 pt-0.5" data-testid="nisar-product-level-selector">
                            <div className="flex items-center justify-between gap-2">
                              <span className="text-xs text-sp-text-secondary whitespace-nowrap">
                                {S.nisarLevelTitle}
                              </span>
                              <span className="font-mono text-xs font-medium text-sp-text whitespace-nowrap">
                                {activeNisarLevel} · {activeLevelMeta?.scienceLevel || "L1"}
                              </span>
                            </div>
                            <div className="grid grid-cols-3 gap-1">
                              {NISAR_OFFICIAL_PRODUCT_LEVELS.map((opt) => {
                                const isSelected = activeNisarLevel === opt.value;
                                const optDesc =
                                  locale === "zh-CN"
                                    ? opt.descriptionZh
                                    : `${opt.label} (${opt.scienceLevel})`;
                                return (
                                  <button
                                    key={opt.value}
                                    type="button"
                                    data-testid={`nisar-level-pill-${opt.value}`}
                                    aria-pressed={isSelected}
                                    title={
                                      isNisarSBand && !opt.sBandActive
                                        ? `${opt.label} (${optDesc})${S.onlyLBandNote}`
                                        : `${opt.label} · ${optDesc}`
                                    }
                                    onClick={() => {
                                      updateWorkbenchToolOptions({ nisarProductLevel: opt.value });
                                      onProductTypeChange(opt.value);
                                    }}
                                    className={cn(
                                      "h-[26px] px-1.5 rounded-sp-sm border text-xs font-mono transition-colors cursor-pointer select-none inline-flex items-center justify-center whitespace-nowrap",
                                      isSelected
                                        ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                        : isNisarSBand && !opt.sBandActive
                                          ? "border-sp-line bg-sp-surface-muted/70 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover"
                                          : "border-sp-line bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                                    )}
                                  >
                                    <span>{opt.label}</span>
                                  </button>
                                );
                              })}
                            </div>
                            {!isNisarSBand && (() => {
                              const currentTiers: ("validated" | "provisional")[] =
                                Array.isArray(toolOptions.nisarReleaseTiers) && toolOptions.nisarReleaseTiers.length > 0
                                  ? toolOptions.nisarReleaseTiers
                                  : ["validated", "provisional"];
                              const toggleTier = (tier: "validated" | "provisional") => {
                                const has = currentTiers.includes(tier);
                                if (has && currentTiers.length <= 1) return;
                                const next = has
                                  ? currentTiers.filter((x) => x !== tier)
                                  : [...currentTiers, tier];
                                updateWorkbenchToolOptions({ nisarReleaseTiers: next });
                              };
                              return (
                                <div className="flex flex-col gap-1.5 pt-1" data-testid="nisar-release-tier-selector">
                                  <span className="text-xs text-sp-text-secondary whitespace-nowrap">
                                    {S.releaseTierTitle}
                                  </span>
                                  <div className="grid grid-cols-2 gap-1">
                                    <button
                                      type="button"
                                      role="checkbox"
                                      aria-checked={currentTiers.includes("validated")}
                                      data-testid="nisar-tier-checkbox-validated"
                                      title={S.validatedTitle}
                                      onClick={() => toggleTier("validated")}
                                      className={cn(
                                        "h-[26px] px-1.5 rounded-sp-sm border text-xs font-mono transition-colors cursor-pointer select-none inline-flex items-center justify-center whitespace-nowrap",
                                        currentTiers.includes("validated")
                                          ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                          : "border-sp-line bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                                      )}
                                    >
                                      <span>Validated</span>
                                    </button>
                                    <button
                                      type="button"
                                      role="checkbox"
                                      aria-checked={currentTiers.includes("provisional")}
                                      data-testid="nisar-tier-checkbox-provisional"
                                      title={S.provisionalTitle}
                                      onClick={() => toggleTier("provisional")}
                                      className={cn(
                                        "h-[26px] px-1.5 rounded-sp-sm border text-xs font-mono transition-colors cursor-pointer select-none inline-flex items-center justify-center whitespace-nowrap",
                                        currentTiers.includes("provisional")
                                          ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                          : "border-sp-line bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                                      )}
                                    >
                                      <span>Provisional</span>
                                    </button>
                                  </div>
                                </div>
                              );
                            })()}
                            {isNisarSBand && activeNisarLevel === "SME2" ? (
                              <span
                                data-testid="nisar-sband-level-pending-note"
                                className="text-xs text-sp-warning-text leading-snug"
                              >
                                {S.sBandLevelsNote}
                              </span>
                            ) : isNisarSBand ? (
                              <span
                                data-testid="nisar-sband-level-route-note"
                                className="text-xs text-sp-text-muted leading-snug"
                              >
                                {activeLevelDesc}{S.nisarSBandConnected}
                              </span>
                            ) : activeNisarLevel !== "RSLC" ? (
                              <span
                                data-testid="nisar-lband-level-route-note"
                                className="text-xs text-sp-text-muted leading-snug"
                              >
                                {activeLevelDesc}{S.nisarLBandConnected}
                              </span>
                            ) : (
                              <span className="text-xs text-sp-text-muted leading-snug">
                                {S.nisarLBandRslcOnly}
                              </span>
                            )}
                          </div>
                        );
                      })()}
                    </>
                  )}

                  {/* 光学数据源字段：按 schema 渲染 facet 筛选器 */}
                  {kind === "optical" && opticalSchema && (
                    <>
                      {opticalFacets.map((facet) => {
                        if (facet.control === "multi_enum") {
                          const selected = draftSelection(opticalDraft ?? {}, facet.filter_id);
                          const isPlatformFacet = facet.filter_id === "platform" && facet.options.length === 3;
                          return (
                            <div key={facet.filter_id} className="flex flex-col gap-1">
                              <span className="text-xs text-sp-text-secondary">
                                {localizeSearchFilterLabel(facet)}
                              </span>
                              <div
                                className={cn(
                                  "pt-0.5 w-full",
                                  isPlatformFacet ? "grid grid-cols-3 gap-1" : "flex flex-wrap gap-1.5"
                                )}
                              >
                                {facet.options.map((opt) => {
                                  const optLabel = localizeSearchFilterOptionLabel(opt);
                                  const isSelected = selected.includes(opt.value);
                                  return (
                                    <button
                                      key={opt.value}
                                      type="button"
                                      title={optLabel}
                                      aria-label={optLabel}
                                      onClick={() => {
                                        const next = isSelected
                                          ? selected.filter((v) => v !== opt.value)
                                          : [...selected, opt.value];
                                        onOpticalDraftChange?.({
                                          ...(opticalDraft ?? {}),
                                          [facet.filter_id]: next,
                                        });
                                      }}
                                      className={cn(
                                        "h-[26px] rounded-sp-sm text-xs transition-colors cursor-pointer select-none inline-flex items-center justify-center border min-w-0",
                                        isPlatformFacet ? "px-0.5 tracking-[-0.02em] w-full truncate whitespace-nowrap" : "px-2.5",
                                        isSelected
                                          ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                          : "border-sp-line text-sp-text-secondary bg-sp-card hover:text-sp-text hover:bg-sp-hover"
                                      )}
                                    >
                                      <span className="truncate">{optLabel}</span>
                                    </button>
                                  );
                                })}
                              </div>
                            </div>
                          );
                        }
                        if (facet.control === "fixed") {
                          return null;
                        }
                        const f = facet as any;
                        return (
                          <div key={f.filter_id} className="text-xs text-sp-danger-text">
                            {t("mapWorkbench.conditions.unsupportedControl", { control: f.control })}
                          </div>
                        );
                      })}
                      {/* 固定规格已在数据源卡片副标题体现，不再重复渲染只读灰色标签行 */}
                    </>
                  )}

                  {/* 全球高清卫星与底图瓦片 (auxiliary.xyz-tiles.v1) 专属筛选器 */}
                  {activeResourceId === "auxiliary.xyz-tiles.v1" && (() => {
                    const L = WORKBENCH_LOCALE_TEXT[locale];
                    return (
                    <div className="flex flex-col gap-3 pt-1">
                      <div className="flex flex-col gap-1">
                        <span className="text-xs text-sp-text-secondary">{L.xyz.sourcePickerHint}</span>
                        <SpSelect
                          value={toolOptions.xyzTileSource}
                          onChange={(val) => {
                            const nextMax = getXyzSourceMaxZoom(val);
                            const clipped = normalizeXyzZooms(selectedXyzZooms, Math.min(15, nextMax), nextMax);
                            updateWorkbenchToolOptions({
                              xyzTileSource: val,
                              xyzTileZooms: clipped,
                              xyzTileZoom: clipped[clipped.length - 1] ?? Math.min(15, nextMax),
                              ...(val !== "esri_wayback" ? { xyzWaybackProbedMaxZoom: null } : {}),
                            });
                            if (val !== "esri_wayback") {
                              onPreviewWaybackOverlay?.(null);
                            }
                            if (activeResourceId === "auxiliary.xyz-tiles.v1" && onOpticalDraftChange && opticalDraft) {
                              const {
                                xyz_zooms: _prevZooms,
                                xyz_zoom: _prevZoom,
                                ...rest
                              } = opticalDraft as SearchFilterDraft & {
                                xyz_zooms?: number[];
                                xyz_zoom?: string;
                              };
                              onOpticalDraftChange(rest);
                            }
                          }}
                          options={[
                            ...(
                              [
                                "arcgis_satellite",
                                "esri_wayback",
                                "google_satellite",
                                "google_hybrid",
                                "gaode_satellite",
                                "tianditu_satellite",
                                "arcgis_topo",
                                "opentopomap",
                                "arcgis_street",
                                "google_map",
                                "gaode_map",
                                "tianditu_vector",
                                "osm",
                                "carto_light",
                                "carto_dark",
                              ] as const
                            ).map((id) => ({
                              value: id,
                              label: XYZ_PRESET_BASEMAPS_LOCALE[locale][id] ?? id,
                            })),
                            { value: "custom", label: L.xyz.customLabel },
                          ]}
                          aria-label={L.xyz.sourcePickerAria}
                        />
                      </div>

                      {isWaybackSource && (
                        <WaybackPanel
                          workingAoi={workingAoi}
                          selectedZooms={selectedXyzZooms}
                          activeVersionId={toolOptions.xyzWaybackVersionId}
                          selectedPeriods={toolOptions.xyzWaybackSelectedPeriods}
                          probedMaxZoom={toolOptions.xyzWaybackProbedMaxZoom}
                          onChangeActiveVersionId={(versionId) =>
                            updateWorkbenchToolOptions({ xyzWaybackVersionId: versionId })
                          }
                          onChangeSelectedPeriods={(periods) =>
                            updateWorkbenchToolOptions({ xyzWaybackSelectedPeriods: periods })
                          }
                          onChangeProbedMaxZoom={(maxZoom) =>
                            updateWorkbenchToolOptions({ xyzWaybackProbedMaxZoom: maxZoom })
                          }
                          onPreviewWaybackOverlay={onPreviewWaybackOverlay}
                        />
                      )}

                      {toolOptions.xyzTileSource === "custom" && (
                        <div className="flex flex-col gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted p-2.5">
                          <div className="flex flex-col gap-1">
                            <span className="text-xs font-medium text-sp-text">{L.xyz.customUrlLabel}</span>
                            <input
                              type="text"
                              placeholder={L.xyz.customUrlPlaceholder}
                              value={toolOptions.xyzTileCustomUrl}
                              onChange={(e) => updateWorkbenchToolOptions({ xyzTileCustomUrl: e.target.value })}
                              className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1 focus:ring-sp-accent"
                            />
                            <span className="text-xs text-sp-text-muted leading-snug">
                              {L.xyz.placeholderHelp}
                              <code className="font-mono">{"{z}"}</code> z · <code className="font-mono">{"{x}"}</code> x · <code className="font-mono">{"{y}"}</code> y · <code className="font-mono">{"{-y}"}</code> TMS · <code className="font-mono">{"{s}"}</code> subdomain (a/b/c).
                            </span>
                          </div>
                          <div className="flex flex-col gap-1">
                            <span className="text-xs text-sp-text-secondary">{L.xyz.crsLabel}</span>
                            <SpSelect
                              value={toolOptions.xyzTileCoordSys}
                              onChange={(val) =>
                                updateWorkbenchToolOptions({
                                  xyzTileCoordSys: val as "auto" | "wgs84" | "gcj02",
                                })
                              }
                              options={[
                                { value: "auto", label: L.xyz.crsAuto },
                                { value: "wgs84", label: L.xyz.crsWgs84 },
                                { value: "gcj02", label: L.xyz.crsGcj02 },
                              ]}
                              aria-label={L.xyz.crsAria}
                            />
                          </div>
                        </div>
                      )}

                      <div className="flex flex-col gap-2">
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-xs text-sp-text-secondary">
                            {L.xyz.zoomHeading.replace("{maxZoom}", String(xyzSourceMaxZoom))}
                          </span>
                          <span className="font-mono text-xs text-sp-text-muted">
                            {selectedXyzZooms.length > 0
                              ? L.xyz.selectedPrefix.replace("{zooms}", selectedXyzZooms.map((z) => `Z${z}`).join(" / "))
                              : L.xyz.noneSelected}
                          </span>
                        </div>
                        <div className="flex flex-wrap items-center gap-1">
                          {[
                            { id: "z10-14", label: "z10~14", zooms: [10, 11, 12, 13, 14] },
                            { id: "z14-18", label: "z14~18", zooms: [14, 15, 16, 17, 18] },
                            { id: "z15-19", label: "z15~19", zooms: [15, 16, 17, 18, 19] },
                          ].map((preset) => {
                            const presetMax = preset.zooms[preset.zooms.length - 1] ?? 22;
                            const presetDisabled = presetMax > xyzSourceMaxZoom;
                            return (
                              <button
                                key={preset.id}
                                type="button"
                                data-testid={`xyz-zoom-preset-${preset.id}`}
                                disabled={presetDisabled}
                                title={
                                  presetDisabled
                                    ? L.xyz.zoomUnsupportedTooltip
                                        .replace("{maxZoom}", String(xyzSourceMaxZoom))
                                        .replace("{presetMax}", String(presetMax))
                                    : preset.label
                                }
                                onClick={() => setXyzZooms(preset.zooms)}
                                className={cn(
                                  "h-6 px-2 rounded-sp-sm border text-xs transition-colors",
                                  presetDisabled
                                    ? "border-sp-line/60 bg-sp-surface-muted text-sp-text-disabled opacity-50 cursor-not-allowed"
                                    : "border-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer",
                                )}
                              >
                                {preset.label}
                              </button>
                            );
                          })}
                          <button
                            type="button"
                            data-testid="xyz-zoom-clear"
                            onClick={() => setXyzZooms([])}
                            className="ml-auto h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-muted hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                          >
                            {L.xyz.clearBtn}
                          </button>
                        </div>
                        <div className="grid grid-cols-6 gap-1">
                          {Array.from({ length: 22 }, (_, i) => i + 1).map((z) => {
                            const unsupported = z > xyzSourceMaxZoom;
                            const active = !unsupported && selectedXyzZooms.includes(z);
                            return (
                              <button
                                key={z}
                                type="button"
                                data-testid={`xyz-zoom-btn-${z}`}
                                aria-pressed={active}
                                disabled={unsupported}
                                title={
                                  unsupported
                                    ? L.xyz.zoomCappedTooltip
                                        .replace("{maxZoom}", String(xyzSourceMaxZoom))
                                        .replace("{z}", String(z))
                                    : `Z${z}`
                                }
                                onClick={() => {
                                  if (unsupported) return;
                                  const next = active
                                    ? selectedXyzZooms.filter((item) => item !== z)
                                    : [...selectedXyzZooms, z];
                                  setXyzZooms(next);
                                }}
                                className={cn(
                                  "h-[26px] rounded-sp-sm text-xs transition-colors select-none inline-flex items-center justify-center border font-mono",
                                  unsupported
                                    ? "border-sp-line/60 bg-sp-surface-muted text-sp-text-disabled opacity-45 cursor-not-allowed"
                                    : active
                                      ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold cursor-pointer"
                                      : "border-sp-line text-sp-text-secondary bg-sp-card hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                                )}
                              >
                                Z{z}
                              </button>
                            );
                          })}
                        </div>
                        <div
                          data-testid="xyz-tile-estimator-card"
                          className="rounded-sp-sm border border-sp-line bg-sp-surface-muted p-2.5 flex flex-col gap-1.5"
                        >
                          <div className="flex items-center justify-between gap-2 text-xs font-medium text-sp-text">
                            <span className="whitespace-nowrap">{L.xyz.estimateHeading}</span>
                            {xyzEstimate && (
                              <span className="font-mono text-xs text-sp-text-muted tabular-nums whitespace-nowrap">
                                {L.xyz.zoomCount.replace("{count}", String(xyzEstimate.zooms.length))}
                              </span>
                            )}
                          </div>
                          {!workingAoi ? (
                            <span className="text-xs text-sp-text-muted leading-snug">
                              {L.xyz.selectAoiFirst}
                            </span>
                          ) : selectedXyzZooms.length === 0 ? (
                            <span className="text-xs text-sp-warning-text leading-snug">
                              {L.xyz.selectZoomFirst.replace("{maxZoom}", String(xyzSourceMaxZoom))}
                            </span>
                          ) : xyzEstimate ? (
                            (() => {
                              const scaledBatch =
                                isWaybackSource && waybackPeriodCount > 1
                                  ? scaleWaybackEstimateForPeriods(xyzEstimate, waybackPeriodCount)
                                  : null;
                              const shownTiles = scaledBatch ? scaledBatch.totalTiles : xyzEstimate.totalTiles;
                              const shownNetBytes = scaledBatch
                                ? scaledBatch.totalNetworkDownloadBytes
                                : xyzEstimate.totalNetworkDownloadBytes;
                              const shownOutBytes = scaledBatch
                                ? scaledBatch.totalEstimatedOutputBytes
                                : xyzEstimate.totalEstimatedOutputBytes;
                              return (
                                <>
                                  <div className="grid grid-cols-1 gap-1 text-xs">
                                    <div className="flex items-center justify-between gap-2">
                                      <span className="text-sp-text-muted whitespace-nowrap shrink-0">
                                        {scaledBatch
                                          ? L.xyz.tileCountCumulative.replace("{count}", String(scaledBatch.periodCount))
                                          : L.xyz.tileCountSingle}
                                      </span>
                                      <span className="font-mono font-semibold text-sp-text tabular-nums whitespace-nowrap">
                                        {L.xyz.perPeriodCap
                                          .replace("{shown}", formatTileCount(shownTiles))
                                          .replace("{cap}", formatTileCount(xyzEstimate.maxTilesLimit))}
                                      </span>
                                    </div>
                                    <div className="flex items-center justify-between gap-2">
                                      <span className="text-sp-text-muted whitespace-nowrap shrink-0">{L.xyz.gridSizeLabel}</span>
                                      <span className="font-mono text-sp-text tabular-nums whitespace-nowrap">
                                        {xyzEstimate.maxWidthPx.toLocaleString()} × {xyzEstimate.maxHeightPx.toLocaleString()} px
                                      </span>
                                    </div>
                                    <div className="flex items-center justify-between gap-2">
                                      <span className="text-sp-text-muted whitespace-nowrap shrink-0">{L.xyz.estTrafficLabel}</span>
                                      <span className="font-mono text-sp-text tabular-nums whitespace-nowrap">
                                        {formatXyzByteSize(shownNetBytes)}
                                      </span>
                                    </div>
                                    <div className="flex items-center justify-between gap-2">
                                      <span className="text-sp-text-muted whitespace-nowrap shrink-0">{L.xyz.estFileSizeLabel}</span>
                                      <span className="font-mono font-semibold text-sp-text tabular-nums whitespace-nowrap">
                                        {formatXyzByteSize(shownOutBytes)}
                                      </span>
                                    </div>
                                    <div className="flex items-center justify-between gap-2">
                                      <span className="text-sp-text-muted whitespace-nowrap shrink-0">{L.xyz.estMemoryLabel}</span>
                                      <span className="font-mono text-sp-text tabular-nums whitespace-nowrap">
                                        {L.xyz.streamingMemValue.replace("{mem}", formatXyzByteSize(xyzEstimate.estimatedPeakMemoryBytes))}
                                      </span>
                                    </div>
                                  </div>
                                  <span className="text-xs text-sp-text-muted leading-snug">
                                    {L.xyz.bigTiffNote.replace(
                                      "{cog}",
                                      xyzEstimate.needsCogStagingDisk ? L.xyz.cogStagingSuffix : "",
                                    )}
                                  </span>
                                  {xyzEstimate.exceedsHardLimit && (
                                    <span
                                      data-testid="xyz-tile-hard-limit-notice"
                                      role="alert"
                                      className="text-xs font-medium text-sp-danger-text leading-snug"
                                    >
                                      {L.xyz.limitExceededErr
                                        .replace("{total}", formatTileCount(xyzEstimate.totalTiles))
                                        .replace("{cap}", formatTileCount(xyzEstimate.maxTilesLimit))}
                                    </span>
                                  )}
                                  {xyzEstimate.exceedsJpegDimensionLimit && (
                                    <span
                                      data-testid="xyz-tile-jpeg-limit-notice"
                                      role="alert"
                                      className="text-xs font-medium text-sp-danger-text leading-snug"
                                    >
                                      {L.xyz.jpegLimitErr
                                        .replace("{w}", xyzEstimate.maxGridWidthPx.toLocaleString())
                                        .replace("{h}", xyzEstimate.maxGridHeightPx.toLocaleString())}
                                    </span>
                                  )}
                                </>
                              );
                            })()
                          ) : null}
                        </div>
                        <span className="text-xs text-sp-text-muted leading-snug">
                          {L.xyz.zoomSupportNote.replace("{maxZoom}", String(xyzSourceMaxZoom))}
                        </span>
                      </div>

                      {(() => {
                        const xyzExportFormatOptions = (["gtiff", "cog", "mbtiles", "zip", "png", "jpg", "envi", "vrt"] as const).map(
                          (val) => ({
                            value: val,
                            label: XYZ_EXPORT_FORMAT_LOCALE[locale][val] ?? val,
                          }),
                        );
                        const xyzCompressionOptions = (["DEFLATE", "LZW", "JPEG", "WEBP", "NONE"] as const).map(
                          (val) => ({
                            value: val,
                            label: XYZ_COMPRESSION_LOCALE[locale][val] ?? val,
                          }),
                        );
                        const activeXyzExportFormatLabel =
                          xyzExportFormatOptions.find((o) => o.value === toolOptions.xyzTileExportFormat)?.label ??
                          toolOptions.xyzTileExportFormat.toUpperCase();
                        const activeXyzCompressionLabel =
                          xyzCompressionOptions.find((o) => o.value === toolOptions.xyzTileCompression)?.label ??
                          toolOptions.xyzTileCompression;

                        return (
                          <>
                            <div
                              data-testid="xyz-export-format-field"
                              className="flex flex-col gap-1"
                              title={L.xyz.exportFormatTitle.replace("{label}", activeXyzExportFormatLabel)}
                            >
                              <span className="text-xs text-sp-text-secondary whitespace-nowrap">{L.xyz.exportFormatLabel}</span>
                              <SpSelect
                                value={toolOptions.xyzTileExportFormat}
                                onChange={(val) =>
                                  updateWorkbenchToolOptions({
                                    xyzTileExportFormat: val as WorkbenchToolOptionsSnapshot["xyzTileExportFormat"],
                                  })
                                }
                                options={xyzExportFormatOptions}
                                aria-label={L.xyz.exportFormatAria}
                              />
                            </div>

                            {toolOptions.xyzTileExportFormat !== "mbtiles" && (
                              <label className="flex items-start gap-2 cursor-pointer select-none rounded-sp-sm border border-sp-line bg-sp-card p-2">
                                <input
                                  type="checkbox"
                                  checked={toolOptions.xyzTileStitchMosaic}
                                  onChange={(e) => updateWorkbenchToolOptions({ xyzTileStitchMosaic: e.target.checked })}
                                  className="mt-0.5 h-4 w-4 rounded-sp-xs border-sp-line text-sp-accent accent-sp-accent"
                                />
                                <div className="flex flex-col gap-0.5">
                                  <span className="text-xs font-medium text-sp-text">{L.xyz.stitchToggleTitle}</span>
                                  <span className="text-xs text-sp-text-muted leading-snug">
                                    {L.xyz.stitchToggleDesc}
                                  </span>
                                </div>
                              </label>
                            )}

                            {toolOptions.xyzTileExportFormat !== "mbtiles" && toolOptions.xyzTileStitchMosaic && (
                              <div className="flex flex-col gap-2.5 rounded-sp-sm border border-sp-line bg-sp-surface-muted p-2.5">
                                <div className="grid grid-cols-1 gap-2">
                                  <div
                                    data-testid="xyz-compression-field"
                                    className="flex flex-col gap-1"
                                    title={L.xyz.compressionTitle.replace("{label}", activeXyzCompressionLabel)}
                                  >
                                    <span className="text-xs text-sp-text-secondary whitespace-nowrap">{L.xyz.compressionLabel}</span>
                                    <SpSelect
                                      value={toolOptions.xyzTileCompression}
                                      onChange={(val) =>
                                        updateWorkbenchToolOptions({
                                          xyzTileCompression: val as WorkbenchToolOptionsSnapshot["xyzTileCompression"],
                                        })
                                      }
                                      options={xyzCompressionOptions}
                                      aria-label={L.xyz.compressionAria}
                                    />
                                  </div>
                                </div>

                                <label className="flex items-center gap-2 cursor-pointer select-none text-xs text-sp-text">
                                  <input
                                    type="checkbox"
                                    checked={toolOptions.xyzTileBuildPyramids}
                                    onChange={(e) =>
                                      updateWorkbenchToolOptions({ xyzTileBuildPyramids: e.target.checked })
                                    }
                                    className="h-3.5 w-3.5 rounded-sp-xs border-sp-line text-sp-accent accent-sp-accent"
                                  />
                                  <span>{L.xyz.buildOverviewsLabel}</span>
                                </label>
                              </div>
                            )}
                          </>
                        );
                      })()}
                      </div>
                    );
                  })()}

                    {/* Google Earth Engine (GEE) 专属筛选器与 4 类动态筛选模板 */}
                    {activeResourceId.startsWith("science.gee.") && (() => {
                      const G = WORKBENCH_LOCALE_TEXT[locale].geeInline;
                      const resolvedAssetId =
                        toolOptions.geeAssetId && toolOptions.geeAssetId !== "ALL"
                          ? toolOptions.geeAssetId
                          : CATALOG_METADATA_REGISTRY[activeResourceId]?.geeAssetId ||
                            filterGeePublicCatalog(toolOptions.geeTheme || "all", "")[0]?.assetId ||
                            "COPERNICUS/S2_SR_HARMONIZED";
                      const rawEntryMeta = lookupGeePublicDataset(resolvedAssetId);
                      const entryMeta = rawEntryMeta ? localizeGeeDatasetEntry(rawEntryMeta, locale) : null;
                      const filterTemplate = entryMeta?.filterTemplate ?? (isStaticGeeAssetId(resolvedAssetId) ? "static" : "optical");
                      const bandOptions = (
                        entryMeta?.selectableBands?.length
                          ? entryMeta.selectableBands
                          : [{ id: "", label: G.defaultBandsOption }]
                      ).map((b) => ({ value: b.id, label: b.label }));
                      const activeBandVal = bandOptions.some((o) => o.value === (toolOptions.geeSelectedBand ?? ""))
                        ? (toolOptions.geeSelectedBand ?? "")
                        : bandOptions[0].value;

                      return (
                        <div className="flex flex-col gap-2">
                          <GeeDatasetSelectorSection
                            assetId={
                              toolOptions.geeAssetId && toolOptions.geeAssetId !== "ALL"
                                ? toolOptions.geeAssetId
                                : (CATALOG_METADATA_REGISTRY[activeResourceId]?.geeAssetId || "ALL")
                            }
                            theme={toolOptions.geeTheme || "all"}
                            onChangeTheme={(nextTheme) => {
                              updateWorkbenchToolOptions({
                                geeTheme: nextTheme,
                                geeAssetId: "ALL",
                                geeSelectedPresetId: "science.gee.catalog.v1",
                                geeSelectedBand: "",
                              });
                            }}
                            onChangeAssetId={(nextAssetId) => {
                              const matchedPreset = Object.entries(CATALOG_METADATA_REGISTRY).find(
                                ([resId, meta]) =>
                                  resId !== "science.gee.catalog.v1" &&
                                  resId.startsWith("science.gee.") &&
                                  meta.geeAssetId === nextAssetId
                              );
                              updateWorkbenchToolOptions({
                                geeAssetId: nextAssetId || "ALL",
                                geeSelectedPresetId: matchedPreset ? matchedPreset[0] : "science.gee.catalog.v1",
                                geeSelectedBand: "",
                              });
                            }}
                          />

                          {/* 动态参数模板 (Optical / SAR / Climate-Env / Static) */}
                          <div
                            data-testid="gee-dynamic-filter-template"
                            data-template={filterTemplate}
                            className="flex flex-col gap-2 pt-1.5 border-t border-sp-line/70"
                          >
                            {filterTemplate === "optical" && (
                              <div className="flex flex-col gap-1.5">
                                <div className="flex items-center justify-between gap-2">
                                  <span className="text-xs text-sp-text-secondary whitespace-nowrap">
                                    {G.maxCloudLabel}
                                  </span>
                                  <span className="font-mono text-xs text-sp-text-muted">
                                    &lt;= {toolOptions.geeMaxCloud ?? 30}%
                                  </span>
                                </div>
                                <SpSelect
                                  value={String(toolOptions.geeMaxCloud ?? 30)}
                                  onChange={(v) =>
                                    updateWorkbenchToolOptions({ geeMaxCloud: Number(v) || 30 })
                                  }
                                  options={[
                                    { value: "10", label: G.cloudOpt10 },
                                    { value: "20", label: G.cloudOpt20 },
                                    { value: "30", label: G.cloudOpt30 },
                                    { value: "50", label: G.cloudOpt50 },
                                    { value: "80", label: G.cloudOpt80 },
                                    { value: "100", label: G.cloudOpt100 },
                                  ]}
                                  aria-label={G.maxCloudLabel}
                                />
                              </div>
                            )}

                            {filterTemplate === "sar" && (
                              <div className="flex flex-col gap-1.5">
                                <span className="text-xs text-sp-text-secondary">
                                  {G.polarizationLabel}
                                </span>
                                <SpSelect
                                  value={toolOptions.geeSarPolarization || "ALL"}
                                  onChange={(v) =>
                                    updateWorkbenchToolOptions({ geeSarPolarization: v })
                                  }
                                  options={[
                                    { value: "ALL", label: G.polAll },
                                    { value: "VV", label: G.polVv },
                                    { value: "VH", label: G.polVh },
                                    { value: "HH", label: G.polHh },
                                    { value: "HV", label: G.polHv },
                                  ]}
                                  aria-label={G.polarizationAria}
                                />
                              </div>
                            )}

                            {filterTemplate === "climate_env" && (
                              <div className="flex flex-col gap-1.5">
                                <span className="text-xs text-sp-text-secondary">
                                  {G.temporalReducerLabel}
                                </span>
                                <SpSelect
                                  value={toolOptions.geeTemporalStat || "median"}
                                  onChange={(v) =>
                                    updateWorkbenchToolOptions({ geeTemporalStat: v })
                                  }
                                  options={[
                                    { value: "median", label: G.reducerMedian },
                                    { value: "mean", label: G.reducerMean },
                                    { value: "max", label: G.reducerMax },
                                    { value: "min", label: G.reducerMin },
                                  ]}
                                  aria-label={G.temporalReducerAria}
                                />
                              </div>
                            )}

                            {filterTemplate === "static" && (
                              <div className="rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2.5 py-1.5 text-xs text-sp-text-secondary leading-relaxed">
                                <span>
                                  {entryMeta?.coverageNote || G.staticAssetNote}
                                </span>
                              </div>
                            )}

                            <div className="flex flex-col gap-1">
                              <span className="text-xs text-sp-text-secondary">
                                {G.targetBandLabel}
                              </span>
                              <SpSelect
                                value={activeBandVal}
                                onChange={(v) => updateWorkbenchToolOptions({ geeSelectedBand: v })}
                                options={bandOptions}
                                aria-label={G.targetBandAria}
                              />
                            </div>
                          </div>

                          <button
                            type="button"
                            data-testid="gee-enter-workbench-btn"
                            onClick={() => onScientificModeChange("gee")}
                            className="eo-tactile-secondary h-7 w-full rounded-sp-sm border border-sp-line bg-sp-card px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                          >
                            {G.openGeeWorkbenchBtn}
                          </button>
                        </div>
                      );
                    })()}

                    {/* Sentinel-1 精密轨道：支持主动按 AOI/日期下载与匹配本地影像文件夹轨道 */}
                    {activeResourceId === "orbit.sentinel-1.poeorb.v1" && (() => {
                      const O = WORKBENCH_LOCALE_TEXT[locale].orbitMode;
                      return (
                      <div
                        data-testid="active-orbit-panel"
                        className="flex flex-col gap-2.5 pt-1 border-t border-sp-line/70"
                      >
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between gap-2">
                            <span className="text-xs text-sp-text-secondary">
                              {O.modeLabel}
                            </span>
                            <span className="rounded-full bg-sp-surface-muted px-2 py-0.5 font-mono text-xs text-sp-text-muted whitespace-nowrap">
                              POEORB / RESORB
                            </span>
                          </div>
                          <div className="flex items-center gap-1 p-1 bg-sp-surface-muted rounded-sp-sm ring-1 ring-sp-line">
                            <button
                              type="button"
                              data-testid="orbit-source-mode-current"
                              onClick={() => handleSelectOrbitSubMode("current")}
                              className={cn(
                                "flex-1 px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                                effectiveOrbitSubMode === "current"
                                  ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                                  : "text-sp-text-secondary hover:text-sp-text"
                              )}
                            >
                              {O.byAoiBtn}
                            </button>
                            <button
                              type="button"
                              data-testid="orbit-source-mode-local"
                              onClick={() => handleSelectOrbitSubMode("local")}
                              className={cn(
                                "flex-1 px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                                effectiveOrbitSubMode === "local"
                                  ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                                  : "text-sp-text-secondary hover:text-sp-text"
                              )}
                            >
                              {O.localFolderBtn}
                            </button>
                          </div>
                        </div>

                        {effectiveOrbitSubMode === "local" ? (
                          <div className="flex flex-col gap-2">
                            <div className="flex flex-col gap-1">
                              <span className="text-xs text-sp-text-secondary">
                                {t("mapWorkbench.conditions.localSlcImageDirectory")}
                              </span>
                              <div className="flex gap-1.5">
                                <input
                                  type="text"
                                  data-testid="orbit-local-slc-path-input"
                                  placeholder={O.localInputPlaceholder}
                                  value={localSlcPath}
                                  onChange={(e) => onChangeLocalSlcPath?.(e.target.value)}
                                  className="h-8 flex-1 min-w-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none"
                                />
                                <button
                                  type="button"
                                  data-testid="orbit-local-slc-browse-btn"
                                  onClick={onPickLocalSlcPath}
                                  className="h-8 px-2 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
                                >
                                  <FolderOpen className="h-3.5 w-3.5" />
                                  <span>{t("mapWorkbench.conditions.browse")}</span>
                                </button>
                                <button
                                  type="button"
                                  data-testid="orbit-local-slc-script-btn"
                                  onClick={async () => {
                                    try {
                                      const picked = await pickOpenFile(O.pickScriptTitle, [
                                        O.pickScriptFilter,
                                      ]);
                                      if (picked && picked.ok && picked.path) {
                                        onChangeLocalSlcPath?.(picked.path);
                                      }
                                    } catch {}
                                  }}
                                  title={O.pickScriptBtnTitle}
                                  className="h-8 px-2 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
                                >
                                  <FileCode className="h-3.5 w-3.5" />
                                  <span>{O.scriptBtn}</span>
                                </button>
                              </div>
                            </div>
                            <div className="rounded-sp-sm bg-sp-surface-muted p-2 text-xs text-sp-text-secondary leading-relaxed">
                              {O.localScanDesc}
                            </div>
                          </div>
                        ) : (
                          <div className="flex flex-col gap-2">
                            <div className="rounded-sp-sm bg-sp-surface-muted p-2 text-xs text-sp-text-secondary leading-relaxed">
                              {O.aoiQueryDesc}
                            </div>
                          </div>
                        )}
                      </div>
                      );
                    })()}

                    {/* GACOS 大气延迟改正：严格对齐 www.gacos.net 官网参数契约（0.05°–10° 跨度、8 位 YYYYMMDD、精确到分钟 UTC、GeoTIFF/Binary） */}
                    {activeResourceId === "atmospheric.gacos.ztd.v1" && (() => {
                      const G = WORKBENCH_LOCALE_TEXT[locale].gacos;
                      return (
                      <div
                        data-testid="gacos-active-panel"
                        className="flex flex-col gap-2.5 pt-1 border-t border-sp-line/70"
                      >
                        {/* 1. 研究区经纬度跨度检查 (0.05°–10°) */}
                        <div
                          data-testid="gacos-aoi-span-check"
                          className={cn(
                            "rounded-sp-sm border px-2.5 py-2 text-xs leading-snug flex flex-col gap-1",
                            gacosAudit.spanStatus === "too_large"
                              ? "border-sp-danger-text/40 bg-sp-danger-bg text-sp-danger-text"
                              : gacosAudit.spanStatus === "too_small" || gacosAudit.spanStatus === "missing"
                              ? "border-sp-warning-text/40 bg-sp-warning-bg text-sp-warning-text"
                              : "border-sp-line bg-sp-surface-muted text-sp-text-secondary"
                          )}
                        >
                          {gacosAudit.spanStatus === "missing" || !workingAoi ? (
                            <span>
                              {G.bindAoiNotice}
                            </span>
                          ) : (
                            <>
                              <div
                                data-testid="gacos-aoi-coords-grid"
                                className="grid grid-cols-2 gap-x-3 gap-y-0.5 font-mono text-xs tabular-nums text-sp-text"
                              >
                                <span className="whitespace-nowrap">{formatSignedLon(workingAoi.west, 3)}</span>
                                <span className="whitespace-nowrap">{formatSignedLon(workingAoi.east, 3)}</span>
                                <span className="whitespace-nowrap">{formatSignedLat(workingAoi.south, 3)}</span>
                                <span className="whitespace-nowrap">{formatSignedLat(workingAoi.north, 3)}</span>
                              </div>
                              <div className="flex flex-wrap items-center justify-between gap-1">
                                <span className="font-mono">
                                  {G.spanPrefix
                                    .replace("{lon}", gacosAudit.lonSpanDeg.toFixed(2))
                                    .replace("{lat}", gacosAudit.latSpanDeg.toFixed(2))}
                                </span>
                                <span className="font-medium">
                                  {gacosAudit.spanStatus === "too_large"
                                    ? G.spanTooLarge
                                    : gacosAudit.spanStatus === "too_small"
                                    ? G.spanTooSmall
                                    : G.spanValid}
                                </span>
                              </div>
                            </>
                          )}
                        </div>

                        {/* 2. 8 位 YYYYMMDD 日期列表 */}
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between gap-2">
                            <span className="text-xs text-sp-text-secondary">
                              {G.datesHeading}
                            </span>
                            <span className="text-xs text-sp-text-muted whitespace-nowrap">
                              {G.batchCapBadge}
                            </span>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <input
                              type="date"
                              data-testid="gacos-date-picker-helper"
                              aria-label={G.pickDateAria}
                              value={gacosDatePickerValue}
                              onChange={(e) => setGacosDatePickerValue(e.target.value)}
                              className="h-7 flex-1 min-w-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text tabular-nums outline-none"
                            />
                            <button
                              type="button"
                              data-testid="gacos-add-date-btn"
                              disabled={!gacosDatePickerValue}
                              onClick={() => {
                                const compact = gacosDatePickerValue.replace(/-/g, "");
                                if (!/^\d{8}$/.test(compact)) return;
                                const existing = gacosAudit.validDates;
                                const nextText = existing.includes(compact)
                                  ? toolOptions.gacosDatesText
                                  : [toolOptions.gacosDatesText.trim(), compact]
                                      .filter(Boolean)
                                      .join("\n");
                                updateWorkbenchToolOptions({ gacosDatesText: nextText });
                              }}
                              className="h-7 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer shrink-0 whitespace-nowrap"
                            >
                              {G.addDateBtn}
                            </button>
                          </div>
                          {/* 从本地 SLC 目录自动扫描提取影像日期与过境 UTC 时分 */}
                          <div className="flex items-center gap-1.5">
                            <input
                              type="text"
                              data-testid="gacos-slc-dir-input"
                              aria-label={G.pathAria}
                              placeholder={G.pathPlaceholder}
                              value={gacosSlcDir}
                              onChange={(e) => setGacosSlcDir(e.target.value)}
                              className="h-7 flex-1 min-w-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs font-mono text-sp-text placeholder:text-sp-text-disabled outline-none"
                            />
                            <button
                              type="button"
                              data-testid="gacos-browse-slc-dir-btn"
                              onClick={async () => {
                                try {
                                  const picked = await pickDirectory(G.pickSlcDirTitle);
                                  if (picked && picked.ok && picked.path) {
                                    setGacosSlcDir(picked.path);
                                  }
                                } catch {}
                              }}
                              className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer shrink-0 whitespace-nowrap"
                              title={G.dirBtnTitle}
                            >
                              <FolderOpen className="w-3 h-3" />
                              <span>{G.dirBtn}</span>
                            </button>
                            <button
                              type="button"
                              data-testid="gacos-browse-slc-script-btn"
                              onClick={async () => {
                                try {
                                  const picked = await pickOpenFile(G.pickScriptTitle, [
                                    G.pickScriptFilter,
                                  ]);
                                  if (picked && picked.ok && picked.path) {
                                    setGacosSlcDir(picked.path);
                                  }
                                } catch {}
                              }}
                              className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 cursor-pointer shrink-0 whitespace-nowrap"
                              title={G.pickScriptTitle}
                            >
                              <FileCode className="w-3 h-3" />
                              <span>{G.scriptBtn}</span>
                            </button>
                            <button
                              type="button"
                              data-testid="gacos-scan-slc-dir-btn"
                              disabled={!gacosSlcDir.trim() || gacosScanBusy}
                              onClick={async () => {
                                const dir = gacosSlcDir.trim();
                                if (!dir) return;
                                setGacosScanBusy(true);
                                setGacosScanNote(null);
                                try {
                                  const res = (await queryOpenGeodata({
                                    dataset: "scan_local_slc_dates",
                                    code: dir,
                                  })) as {
                                    ok?: boolean;
                                    dates?: string[];
                                    file_count?: number;
                                    dominant_utc_hour?: string | null;
                                    dominant_utc_minute?: string | null;
                                    error?: string;
                                  } | null;
                                  if (!res || !res.ok) {
                                    throw new Error(res?.error || G.scanFailed);
                                  }
                                  const scanned = Array.isArray(res.dates) ? res.dates : [];
                                  if (scanned.length === 0) {
                                    setGacosScanNote({
                                      ok: false,
                                      text: G.noYmdFound,
                                    });
                                    return;
                                  }
                                  const merged = Array.from(
                                    new Set([...gacosAudit.validDates, ...scanned]),
                                  ).sort();
                                  const patch: Record<string, string> = {
                                    gacosDatesText: merged.join("\n"),
                                  };
                                  if (res.dominant_utc_hour) {
                                    patch.gacosUtcHour = String(
                                      Number.parseInt(res.dominant_utc_hour, 10),
                                    );
                                  }
                                  if (res.dominant_utc_minute) {
                                    patch.gacosUtcMinute = String(
                                      Number.parseInt(res.dominant_utc_minute, 10),
                                    );
                                  }
                                  updateWorkbenchToolOptions(patch);
                                  setGacosScanNote({
                                    ok: true,
                                    text: `${G.extractedDatesToast.replace("{count}", String(scanned.length))}${
                                      res.dominant_utc_hour && res.dominant_utc_minute
                                        ? ` · UTC ${res.dominant_utc_hour}:${res.dominant_utc_minute}`
                                        : ""
                                    }`,
                                  });
                                } catch (err) {
                                  setGacosScanNote({
                                    ok: false,
                                    text: err instanceof Error ? err.message : String(err),
                                  });
                                } finally {
                                  setGacosScanBusy(false);
                                }
                              }}
                              className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer shrink-0 whitespace-nowrap"
                            >
                              {gacosScanBusy ? G.scanningBtn : G.extractDatesBtn}
                            </button>
                          </div>
                          {gacosScanNote && (
                            <div
                              data-testid="gacos-scan-note"
                              className={cn(
                                "text-xs leading-snug",
                                gacosScanNote.ok ? "text-sp-success-text" : "text-sp-warning-text",
                              )}
                            >
                              {gacosScanNote.text}
                            </div>
                          )}
                          <textarea
                            data-testid="gacos-dates-textarea"
                            aria-label={G.textareaAria}
                            rows={3}
                            placeholder={G.textareaPlaceholder}
                            value={toolOptions.gacosDatesText}
                            onChange={(e) =>
                              updateWorkbenchToolOptions({ gacosDatesText: e.target.value })
                            }
                            className="w-full rounded-sp-sm border border-sp-line bg-sp-surface p-2 font-mono text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1 focus:ring-sp-accent"
                          />
                          <div
                            data-testid="gacos-dates-summary"
                            className={cn(
                              "text-xs leading-snug",
                              gacosAudit.invalidTokens.length > 0
                                ? "text-sp-danger-text font-medium"
                                : "text-sp-text-muted"
                            )}
                          >
                            {gacosAudit.invalidTokens.length > 0
                              ? G.invalidDateFmt.replace("{tokens}", gacosAudit.invalidTokens.slice(0, 3).join(", "))
                              : G.validDatesSummary
                                  .replace("{count}", String(gacosAudit.validDates.length))
                                  .replace(
                                    "{suffix}",
                                    gacosAudit.batchCount > 1
                                      ? G.splitBatchesNote.replace("{count}", String(gacosAudit.batchCount))
                                      : G.autoSplitNote,
                                  )}
                          </div>
                        </div>

                        {/* 3. 精确到分钟的卫星过境时刻 UTC (H: 00–23, M: 00–59) */}
                        <div className="grid grid-cols-2 gap-2">
                          <div className="flex flex-col gap-1">
                            <span className="text-xs text-sp-text-secondary">{G.utcHourLabel}</span>
                            <SpSelect
                              data-testid="gacos-utc-hour"
                              aria-label={G.utcHourAria}
                              value={String(gacosAudit.utcHourNum)}
                              onChange={(val) =>
                                updateWorkbenchToolOptions({ gacosUtcHour: val })
                              }
                              size="sm"
                              options={Array.from({ length: 24 }, (_, h) => ({
                                value: String(h),
                                label: G.utcHourUnit.replace("{h}", String(h).padStart(2, "0")),
                              }))}
                            />
                          </div>
                          <div className="flex flex-col gap-1">
                            <span className="text-xs text-sp-text-secondary">{G.utcMinLabel}</span>
                            <SpSelect
                              data-testid="gacos-utc-minute"
                              aria-label={G.utcMinAria}
                              value={String(gacosAudit.utcMinuteNum)}
                              onChange={(val) =>
                                updateWorkbenchToolOptions({ gacosUtcMinute: val })
                              }
                              size="sm"
                              options={Array.from({ length: 60 }, (_, m) => ({
                                value: String(m),
                                label: G.utcMinUnit.replace("{m}", String(m).padStart(2, "0")),
                              }))}
                            />
                          </div>
                        </div>

                        {/* 4. 官方数据输出格式选择 */}
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{G.formatLabel}</span>
                          <div
                            data-testid="gacos-output-format"
                            role="radiogroup"
                            aria-label={G.formatAria}
                            className="grid grid-cols-2 gap-1.5"
                          >
                            {[
                              {
                                val: "geotiff" as const,
                                testId: "gacos-format-geotiff",
                                label: "GeoTIFF (.tif)",
                              },
                              {
                                val: "binary" as const,
                                testId: "gacos-format-binary",
                                label: "Binary (.ztd+.rsc)",
                              },
                            ].map((fmt) => {
                              const active = toolOptions.gacosOutputFormat === fmt.val;
                              return (
                                <button
                                  key={fmt.val}
                                  type="button"
                                  role="radio"
                                  aria-checked={active}
                                  data-testid={fmt.testId}
                                  onClick={() =>
                                    updateWorkbenchToolOptions({ gacosOutputFormat: fmt.val })
                                  }
                                  className={cn(
                                    "h-8 px-2 rounded-sp-sm text-xs border transition-colors cursor-pointer whitespace-nowrap",
                                    active
                                      ? "bg-sp-selected border-sp-text text-sp-text font-semibold"
                                      : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                                  )}
                                >
                                  {fmt.label}
                                </button>
                              );
                            })}
                          </div>
                        </div>

                        <div className="text-xs text-sp-text-muted leading-snug">
                          {G.asyncEmailNotice}
                        </div>
                      </div>
                      );
                    })()}

                    {/* 衍生 SAR 干涉成品说明（抑制原始 SLC 的 POEORB 20 天等待提示） */}
                    {isDerivedSarInterferogram(activeResourceId) && (
                      <div
                        data-testid="derived-sar-product-note"
                        className="rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2.5 py-2 text-xs text-sp-text-secondary leading-relaxed"
                      >
                        {WORKBENCH_LOCALE_TEXT[locale].bottomSummary.deriveRegisteredNote}
                      </div>
                    )}

                  {/* 专题科学/辅助数据源子产品与保存目录筛选器 */}
                  {[
                    "science.esa.worldcover.v1",
                    "science.jrc.gsw.v1",
                    "science.alos.palsar-mosaic.v1",
                    "science.modis.ndvi.v1",
                    "science.aria.s1-gunw.v1",
                    "atmospheric.sentinel-5p.l2.v1",
                    "science.opera.disp-s1.v1",
                    "science.comet.licsar.v1",
                    "event.usgs.earthquake.v1",
                    "geodesy.ngl.gnss.v1",
                    "atmospheric.era5.reanalysis.v1",
                    "ionospheric.igs.gim.v1",
                    "orbit.nisar.oe.v1",
                    "orbit.copernicus.pod.v1",
                    "science.nasa.black-marble.v1",
                    "science.nasa.emit.l2a.v1",
                    "science.nasa.ecostress.l2t.v1",
                    "science.dlr.enmap.l2a.v1",
                    "science.aws.sentinel-1-grd.v1",
                    "science.noaa.aws-weather.v1",
                    "science.overture.maps.v1",
                  ].includes(activeResourceId) && (
                    <div className="flex flex-col gap-2.5 pt-1">
                  {/* 专题科学/辅助数据源子产品与保存目录筛选器 */}
                  {activeResourceId.startsWith("science.") ||
                  activeResourceId.startsWith("atmospheric.") ||
                  activeResourceId.startsWith("event.") ||
                  activeResourceId.startsWith("geodesy.") ||
                  activeResourceId.startsWith("ionospheric.") ||
                  activeResourceId.startsWith("orbit.") ? (() => {
                    const T = WORKBENCH_LOCALE_TEXT[locale].topicOptions;
                    return (
                    <>
                      {activeResourceId === "science.esa.worldcover.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.productVersionLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "v200"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "v200", label: T.wcV200 },
                              { value: "v100", label: T.wcV100 },
                            ]}
                            aria-label={T.productVersionAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.jrc.gsw.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.waterLayerLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "occurrence"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "occurrence", label: T.jrcOccurrence },
                              { value: "extent", label: T.jrcExtent },
                              { value: "seasonality", label: T.jrcSeasonality },
                              { value: "change", label: T.jrcChange },
                            ]}
                            aria-label={T.waterLayerAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.alos.palsar-mosaic.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.alosBandLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "HH"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "HH", label: T.alosHh },
                              { value: "HV", label: T.alosHv },
                              { value: "mask", label: T.alosMask },
                            ]}
                            aria-label={T.alosBandAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.modis.ndvi.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.modisBandLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "250m_16_days_NDVI"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "250m_16_days_NDVI", label: T.modisNdvi },
                              { value: "250m_16_days_EVI", label: T.modisEvi },
                              { value: "250m_16_days_VI_Quality", label: T.modisQa },
                            ]}
                            aria-label={T.modisBandAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "atmospheric.sentinel-5p.l2.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.s5pLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "no2"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "no2", label: T.s5pNo2 },
                              { value: "ch4", label: T.s5pCh4 },
                              { value: "so2", label: T.s5pSo2 },
                              { value: "co", label: T.s5pCo },
                              { value: "o3", label: T.s5pO3 },
                              { value: "aer_ai", label: T.s5pAer },
                            ]}
                            aria-label={T.s5pAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.opera.disp-s1.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.operaLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "DISP-S1"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "DISP-S1", label: T.operaDisp },
                              { value: "CSLC-S1", label: T.operaCslc },
                              { value: "RTC-S1", label: T.operaRtc },
                            ]}
                            aria-label={T.operaAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "atmospheric.era5.reanalysis.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.era5Label}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "hourly_insar"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "hourly_insar", label: T.era5HourlyInsar },
                              { value: "hourly_full", label: T.era5HourlyFull },
                              { value: "daily_summary", label: T.era5Daily },
                            ]}
                            aria-label={T.era5Aria}
                          />
                        </div>
                      )}
                      {activeResourceId === "event.usgs.earthquake.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.eqLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "4.5"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "3.0", label: T.eqMw3 },
                              { value: "4.5", label: T.eqMw45 },
                              { value: "5.5", label: T.eqMw55 },
                              { value: "6.5", label: T.eqMw65 },
                            ]}
                            aria-label={T.eqAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "ionospheric.igs.gim.v1" && (
                        <div className="flex flex-col gap-1">
                          <span className="text-xs text-sp-text-secondary">{T.igsLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "COD"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "COD", label: T.igsCod },
                              { value: "IGS", label: T.igsIgs },
                              { value: "JPL", label: T.igsJpl },
                            ]}
                            aria-label={T.igsAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "orbit.nisar.oe.v1" && (
                        <div className="flex flex-col gap-1.5" data-testid="nisar-orbit-oe-panel">
                          <span className="text-xs text-sp-text-secondary">{T.nisarOeLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "ALL"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "ALL", label: T.nisarOeAll },
                              { value: "POE", label: T.nisarOePoe },
                              { value: "MOE", label: T.nisarOeMoe },
                              { value: "NOE", label: T.nisarOeNoe },
                              { value: "FOE", label: T.nisarOeFoe },
                            ]}
                            aria-label={T.nisarOeAria}
                          />
                          <div className="rounded-sp-sm bg-sp-surface-muted px-2 py-1.5 text-xs text-sp-text-secondary leading-snug">
                            {T.nisarOeHint}
                          </div>
                        </div>
                      )}
                      {activeResourceId === "orbit.copernicus.pod.v1" && (
                        <div className="flex flex-col gap-1.5" data-testid="copernicus-pod-orbit-panel">
                          <span className="text-xs text-sp-text-secondary">{T.podLabel}</span>
                          <SpSelect
                            value={toolOptions.openGeodataSubProduct || "ALL"}
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "ALL", label: T.podAll },
                              { value: "S2", label: "Sentinel-2 · AUX_POEORB (.EOF)" },
                              { value: "S3", label: "Sentinel-3 · SR___POE_AX (.SEN3)" },
                            ]}
                            aria-label={T.podAria}
                          />
                          <div className="rounded-sp-sm bg-sp-surface-muted px-2 py-1.5 text-xs text-sp-text-secondary leading-snug">
                            {T.podHint}
                          </div>
                        </div>
                      )}
                      {activeResourceId === "science.nasa.black-marble.v1" && (
                        <div className="flex flex-col gap-1" data-testid="nasa-black-marble-panel">
                          <span className="text-xs text-sp-text-secondary">{T.bmLabel}</span>
                          <SpSelect
                            value={
                              ["VNP46A3", "VNP46A4", "VNP46A2"].includes(toolOptions.openGeodataSubProduct)
                                ? toolOptions.openGeodataSubProduct
                                : "VNP46A3"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "VNP46A3", label: T.bmA3 },
                              { value: "VNP46A4", label: T.bmA4 },
                              { value: "VNP46A2", label: T.bmA2 },
                            ]}
                            aria-label={T.bmAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.nasa.emit.l2a.v1" && (
                        <div className="flex flex-col gap-1" data-testid="nasa-emit-panel">
                          <span className="text-xs text-sp-text-secondary">{T.emitLabel}</span>
                          <SpSelect
                            value={
                              ["EMITL2ARFL", "EMITL2BMIN"].includes(toolOptions.openGeodataSubProduct)
                                ? toolOptions.openGeodataSubProduct
                                : "EMITL2ARFL"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "EMITL2ARFL", label: T.emitL2a },
                              { value: "EMITL2BMIN", label: T.emitL2b },
                            ]}
                            aria-label={T.emitAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.nasa.ecostress.l2t.v1" && (
                        <div className="flex flex-col gap-1" data-testid="nasa-ecostress-panel">
                          <span className="text-xs text-sp-text-secondary">{T.ecoLabel}</span>
                          <SpSelect
                            value={
                              ["ECO_L2T_LSTE", "ECO_L2T_LSTE_EMIS", "ECO_L3T_JET"].includes(
                                toolOptions.openGeodataSubProduct,
                              )
                                ? toolOptions.openGeodataSubProduct
                                : "ECO_L2T_LSTE"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "ECO_L2T_LSTE", label: T.ecoLst },
                              { value: "ECO_L2T_LSTE_EMIS", label: T.ecoEmis },
                              { value: "ECO_L3T_JET", label: T.ecoEt },
                            ]}
                            aria-label={T.ecoAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.dlr.enmap.l2a.v1" && (
                        <div
                          data-testid="dlr-enmap-search-only-note"
                          className="rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2.5 py-2 text-xs text-sp-text-secondary leading-relaxed"
                        >
                          {T.enmapNotePrefix}{" "}
                          <span className="font-mono text-sp-text">https://eoweb.dlr.de</span>
                          {T.enmapNoteSuffix}
                        </div>
                      )}
                      {activeResourceId === "science.aws.sentinel-1-grd.v1" && (
                        <div className="flex flex-col gap-1" data-testid="aws-s1-grd-panel">
                          <span className="text-xs text-sp-text-secondary">{T.s1GrdLabel}</span>
                          <SpSelect
                            value={
                              ["vv", "vh", "hh", "hv"].includes(toolOptions.openGeodataSubProduct)
                                ? toolOptions.openGeodataSubProduct
                                : "vv"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "vv", label: T.s1GrdVv },
                              { value: "vh", label: T.s1GrdVh },
                              { value: "hh", label: T.s1GrdHh },
                              { value: "hv", label: T.s1GrdHv },
                            ]}
                            aria-label={T.s1GrdAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.noaa.aws-weather.v1" && (
                        <div className="flex flex-col gap-1" data-testid="noaa-aws-weather-panel">
                          <span className="text-xs text-sp-text-secondary">{T.noaaLabel}</span>
                          <SpSelect
                            value={
                              ["gfs", "goes19", "goes18"].includes(toolOptions.openGeodataSubProduct)
                                ? toolOptions.openGeodataSubProduct
                                : "gfs"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "gfs", label: T.noaaGfs },
                              { value: "goes19", label: T.noaaGoes19 },
                              { value: "goes18", label: T.noaaGoes18 },
                            ]}
                            aria-label={T.noaaAria}
                          />
                        </div>
                      )}
                      {activeResourceId === "science.overture.maps.v1" && (
                        <div className="flex flex-col gap-1" data-testid="overture-maps-panel">
                          <span className="text-xs text-sp-text-secondary">{T.overtureLabel}</span>
                          <SpSelect
                            value={
                              ["buildings", "places", "transportation", "divisions", "addresses", "base"].includes(
                                toolOptions.openGeodataSubProduct,
                              )
                                ? toolOptions.openGeodataSubProduct
                                : "buildings"
                            }
                            onChange={(val) => updateWorkbenchToolOptions({ openGeodataSubProduct: val })}
                            options={[
                              { value: "buildings", label: T.ovBuildings },
                              { value: "transportation", label: T.ovTransport },
                              { value: "places", label: T.ovPlaces },
                              { value: "divisions", label: T.ovDivisions },
                              { value: "addresses", label: T.ovAddresses },
                              { value: "base", label: T.ovBase },
                            ]}
                            aria-label={T.overtureAria}
                          />
                        </div>
                      )}
                    </>
                    );
                  })() : null}
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* 2. 空间范围 AOI（本地轨道扫描模式与全球全轨历书无需绑定空间范围） */}
            {!(activeResourceId === "orbit.sentinel-1.poeorb.v1" && effectiveOrbitSubMode === "local") &&
              activeResourceId !== "orbit.nisar.oe.v1" &&
              activeResourceId !== "orbit.copernicus.pod.v1" &&
              renderAoiGroup()}

            {/* 3. 时间（XYZ 底图瓦片、本地轨道扫描、GACOS 离散日期、静态 GEE 基准集无需起止日期范围） */}
            {activeResourceId !== "auxiliary.xyz-tiles.v1" &&
              activeResourceId !== "atmospheric.gacos.ztd.v1" &&
              !isStaticGeeAssetSelected &&
              !(activeResourceId === "orbit.sentinel-1.poeorb.v1" && effectiveOrbitSubMode === "local") && (
            <div
              ref={(node) => { groupRefs.current.time = node; }}
              data-condition-group="time"
              data-condition-field="time"
              tabIndex={-1}
              className="bg-sp-surface rounded-[10px] ring-1 ring-sp-line shrink-0 overflow-hidden focus:outline-none"
            >
              <button
                type="button"
                aria-expanded={groupState.time}
                aria-controls="group-time-body"
                onClick={() => handleToggle("time")}
                className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
              >
                {groupState.time ? (
                  <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                )}
                <span>{t("mapWorkbench.conditions.time")}</span>
                {timeInfo ? (
                  <span className="ml-auto text-xs font-normal text-sp-text-muted tabular-nums">
                    {timeInfo}
                  </span>
                ) : null}
              </button>
              {groupState.time && (
                <div id="group-time-body" className="p-3 pt-0 flex flex-col gap-3">
                  {/* 开始日期与结束日期同行两列 */}
                  <div className="grid grid-cols-2 gap-2">
                    <div className="flex flex-col gap-1">
                      <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.startDate")}</span>
                      <input
                        type="date"
                        value={startDate || ""}
                        onChange={(e) => onStartDateChange(e.target.value)}
                        className="h-7 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-1.5 text-xs text-sp-text tabular-nums outline-none focus:ring-1 focus:ring-sp-accent"
                      />
                    </div>
                    <div className="flex flex-col gap-1">
                      <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.endDate")}</span>
                      <input
                        type="date"
                        value={endDate || ""}
                        onChange={(e) => onEndDateChange(e.target.value)}
                        className="h-7 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-1.5 text-xs text-sp-text tabular-nums outline-none focus:ring-1 focus:ring-sp-accent"
                      />
                    </div>
                  </div>
                  {/* 快捷范围按钮 */}
                  <div className="flex flex-wrap gap-1" data-testid="time-quick-presets">
                    {(
                      [
                        { kind: "3m", label: t("mapWorkbench.conditions.recent3Months") },
                        { kind: "6m", label: t("mapWorkbench.conditions.recent6Months") },
                        { kind: "1y", label: t("mapWorkbench.conditions.recentYear") },
                        { kind: "3y", label: t("mapWorkbench.conditions.recent3Years") },
                        { kind: "5y", label: t("mapWorkbench.conditions.recent5Years") },
                      ] as const
                    ).map((preset) => (
                      <button
                        key={preset.kind}
                        type="button"
                        data-testid={`time-quick-${preset.kind}`}
                        title={preset.label}
                        onClick={() => handleQuick(preset.kind)}
                        className="h-6 px-2 text-xs rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer whitespace-nowrap"
                      >
                        {preset.label}
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
            )}
            {/* 4. 云量与质量 (光学模式) */}
            {!isStaticGeeAssetSelected && kind === "optical" && opticalQualityFilters.length > 0 && (
              <div className="bg-sp-surface rounded-sp-md ring-1 ring-sp-line shrink-0 overflow-hidden">
                <button
                  type="button"
                  aria-expanded={groupState.orbit}
                  aria-controls="group-quality-body"
                  onClick={() => handleToggle("orbit")}
                  className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
                >
                  {groupState.orbit ? (
                    <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                  ) : (
                    <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                  )}
                  <span>{t("mapWorkbench.conditions.cloudAndQuality")}</span>
                </button>
                {groupState.orbit && (
                  <div id="group-quality-body" className="p-3 pt-0 flex flex-col gap-3">
                    {opticalQualityFilters.map((qf) => {
                      if (qf.control === "number") {
                        const curVal = draftText(opticalDraft ?? {}, qf.filter_id);
                        return (
                          <div key={qf.filter_id} className="flex flex-col gap-1.5">
                            <div className="flex items-center justify-between">
                              <span className="text-xs text-sp-text-secondary">{localizeSearchFilterLabel(qf)}</span>
                              <div className="relative w-24 flex items-center">
                                <input
                                  type="text"
                                  placeholder={localizeSearchFilterPlaceholder(qf)}
                                  value={curVal}
                                  onChange={(e) => {
                                    onOpticalDraftChange?.({
                                      ...(opticalDraft ?? {}),
                                      [qf.filter_id]: e.target.value,
                                    });
                                  }}
                                  className="h-7 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2 pr-6 text-xs text-sp-text outline-none focus:ring-1 focus:ring-sp-accent tabular-nums"
                                />
                                {qf.unit && (
                                  <span className="absolute right-2 text-xs text-sp-text-muted pointer-events-none">
                                    {qf.unit}
                                  </span>
                                )}
                              </div>
                            </div>
                            <div className="flex items-center gap-1">
                              {[
                                { label: "≤ 10%", val: "10" },
                                { label: "≤ 20%", val: "20" },
                                { label: "≤ 50%", val: "50" },
                                { label: t("mapWorkbench.conditions.noRestriction"), val: "" },
                              ].map((preset) => {
                                const active = curVal.trim() === preset.val;
                                return (
                                  <button
                                    key={preset.label}
                                    type="button"
                                    onClick={() => {
                                      onOpticalDraftChange?.({
                                        ...(opticalDraft ?? {}),
                                        [qf.filter_id]: preset.val,
                                      });
                                    }}
                                    className={cn(
                                      "h-6 px-1.5 whitespace-nowrap flex-1 text-xs rounded-sp-sm transition-colors cursor-pointer",
                                      active
                                        ? "border border-sp-text bg-sp-selected text-sp-text font-semibold"
                                        : "ring-1 ring-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                                    )}
                                  >
                                    {preset.label}
                                  </button>
                                );
                              })}
                            </div>
                          </div>
                        );
                      }
                      const f = qf as any;
                      return (
                        <div key={f.filter_id} className="text-xs text-sp-danger-text">
                          {t("mapWorkbench.conditions.unsupportedControl", { control: f.control })}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            {/* 4. 轨道 / Frame / Polarization (SAR 模式) */}
            {kind === "sar" && (
              <div
                ref={(node) => { groupRefs.current.orbit = node; }}
                data-condition-group="orbit"
                data-condition-field="orbit"
                tabIndex={-1}
                className="bg-sp-surface rounded-[10px] ring-1 ring-sp-line shrink-0 overflow-hidden focus:outline-none"
              >
              <button
                type="button"
                aria-expanded={groupState.orbit}
                aria-controls="group-orbit-body"
                onClick={() => handleToggle("orbit")}
                className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
              >
                {groupState.orbit ? (
                  <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                )}
                <span>{groupOrbitTitle(profile)}</span>
              </button>
              {groupState.orbit && (
                <div id="group-orbit-body" className="p-3 pt-0 flex flex-col gap-3">
                  {(Boolean(profile?.flightDirection) ||
                    activeResourceId === "science.nisar.s.ssar.rslc.v1" ||
                    activeResourceId === "science.opera.disp-s1.v1" ||
                    activeResourceId === "science.aria.s1-gunw.v1" ||
                    activeResourceId === "science.comet.licsar.v1") &&
                    activeResourceId !== "science.alos.palsar-mosaic.v1" && (
                    <div className="flex flex-col gap-1">
                      <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.orbitDirection")}</span>
                      {/* R4a: 移除 min-w-0，自适应文字宽度，确保 Ascending / Descending 不溢出 */}
                      <div className="flex items-center gap-1 p-1 bg-sp-surface-muted rounded-sp-sm ring-1 ring-sp-line">
                        {[
                          { label: t("mapWorkbench.conditions.all"), val: "all" },
                          { label: "Ascending", val: "ascending" },
                          { label: "Descending", val: "descending" },
                        ].map((item) => (
                          <button
                            key={item.val}
                            type="button"
                            onClick={() => onOrbitDirectionChange(item.val)}
                            className={cn(
                              "flex-1 px-1 min-[1600px]:px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                              (orbitDirection || "all") === item.val
                                ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                                : "text-sp-text-secondary hover:text-sp-text"
                            )}
                          >
                            {item.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* Path 与 Frame 同行两列 */}
                  {(profile?.relativeOrbit || profile?.path || profile?.frame) && (
                    <div className="grid grid-cols-2 gap-2">
                      {profile?.relativeOrbit ? (
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between gap-1">
                            <span className="text-xs text-sp-text-secondary">Path</span>
                            {effectiveRelativeOrbitProfile?.min != null && effectiveRelativeOrbitProfile?.max != null && (
                              <span className="text-xs text-sp-text-muted tabular-nums">
                                ({effectiveRelativeOrbitProfile.min}–{effectiveRelativeOrbitProfile.max})
                              </span>
                            )}
                          </div>
                          <input
                            type="text"
                            placeholder={t("mapWorkbench.conditions.optionalUnrestricted")}
                            value={relativeOrbit ?? ""}
                            aria-invalid={Boolean(relativeOrbitError)}
                            onChange={(e) => onRelativeOrbitChange(e.target.value)}
                            className={cn(
                              "h-7 w-full rounded-sp-sm border bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1",
                              relativeOrbitError
                                ? "border-sp-danger-text focus:ring-sp-danger-text"
                                : "border-sp-line focus:ring-sp-accent"
                            )}
                          />
                          {relativeOrbitError && (
                            <span className="text-xs text-sp-danger-text leading-snug">{relativeOrbitError}</span>
                          )}
                        </div>
                      ) : profile?.path ? (
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between gap-1">
                            <span className="text-xs text-sp-text-secondary">Path</span>
                            {effectivePathProfile?.min != null && effectivePathProfile?.max != null && (
                              <span className="text-xs text-sp-text-muted tabular-nums">
                                ({effectivePathProfile.min}–{effectivePathProfile.max})
                              </span>
                            )}
                          </div>
                          <input
                            type="text"
                            placeholder={t("mapWorkbench.conditions.optionalUnrestricted")}
                            value={path ?? ""}
                            aria-invalid={Boolean(pathError)}
                            onChange={(e) => onPathChange?.(e.target.value)}
                            className={cn(
                              "h-7 w-full rounded-sp-sm border bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1",
                              pathError
                                ? "border-sp-danger-text focus:ring-sp-danger-text"
                                : "border-sp-line focus:ring-sp-accent"
                            )}
                          />
                          {pathError && (
                            <span className="text-xs text-sp-danger-text leading-snug">{pathError}</span>
                          )}
                        </div>
                      ) : <div />}

                      {profile?.frame ? (
                        <div className="flex flex-col gap-1">
                          <div className="flex items-center justify-between gap-1">
                            <span className="text-xs text-sp-text-secondary">Frame</span>
                            {!profile.frame.exactWidth && effectiveFrameProfile?.min != null && effectiveFrameProfile?.max != null && (
                              <span className="text-xs text-sp-text-muted tabular-nums">
                                ({effectiveFrameProfile.min}–{effectiveFrameProfile.max})
                              </span>
                            )}
                          </div>
                          <input
                            type="text"
                            placeholder={profile.frame.exactWidth ? t("mapWorkbench.conditions.optionalDigits", { width: profile.frame.exactWidth }) : t("mapWorkbench.conditions.optionalUnrestricted")}
                            value={frame ?? ""}
                            aria-invalid={Boolean(frameError)}
                            onChange={(e) => onFrameChange(e.target.value)}
                            className={cn(
                              "h-7 w-full rounded-sp-sm border bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none focus:ring-1",
                              frameError
                                ? "border-sp-danger-text focus:ring-sp-danger-text"
                                : "border-sp-line focus:ring-sp-accent"
                            )}
                          />
                          {frameError && (
                            <span className="text-xs text-sp-danger-text leading-snug">{frameError}</span>
                          )}
                        </div>
                      ) : <div />}
                    </div>
                  )}

                  {profile?.beamModes && visibleFilterFields(profile, "advanced").some((f) => f.id === "beamModes") && (
                    <div className="flex flex-col gap-1" data-condition-field="beamModes">
                      <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.beamMode")}</span>
                      {profile.beamModes.multiple ? (
                        <div className="flex flex-wrap gap-1.5 pt-0.5">
                          {profile.beamModes.options.map((mode) => {
                            const isSelected = (beamModes || []).includes(mode);
                            return (
                              <button
                                key={mode}
                                type="button"
                                onClick={() => {
                                  const cur = beamModes || [];
                                  const next = isSelected
                                    ? cur.filter((m) => m !== mode)
                                    : [...cur, mode];
                                  onBeamModesChange?.(next);
                                }}
                                className={cn(
                                  "h-[26px] px-2.5 rounded-sp-full text-xs transition-colors cursor-pointer select-none inline-flex items-center justify-center",
                                  isSelected
                                    ? "border border-sp-text bg-sp-selected text-sp-text font-semibold"
                                    : "border border-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                                )}
                              >
                                {mode}
                              </button>
                            );
                          })}
                        </div>
                      ) : (
                        <div className="flex items-center gap-1 p-1 bg-sp-surface-muted rounded-sp-sm ring-1 ring-sp-line">
                          {[
                            { label: t("mapWorkbench.conditions.all"), val: "" },
                            ...profile.beamModes.options.map((m) => ({ label: m, val: m })),
                          ].map((item) => {
                            const currentBeam = (beamModes && beamModes[0]) || "";
                            return (
                              <button
                                key={item.label}
                                type="button"
                                onClick={() => {
                                  const nextBeams = item.val ? [item.val] : [];
                                  onBeamModesChange?.(nextBeams);
                                  if (item.val === "WBS" && polarizations && polarizations.length > 0) {
                                    const allowed = allowedPolarizationsForBeamMode(profile, nextBeams);
                                    const nextPol = polarizations
                                      .map((p) => (p === "HH+HV" && allowed.includes("HH") && !allowed.includes("HH+HV") ? "HH" : p))
                                      .filter((p) => allowed.includes(p));
                                    onPolarizationsChange?.(Array.from(new Set(nextPol)));
                                  }
                                }}
                                className={cn(
                                  "flex-1 px-1 min-[1600px]:px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                                  currentBeam === item.val
                                    ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                                    : "text-sp-text-secondary hover:text-sp-text"
                                )}
                              >
                                {item.label}
                              </button>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  )}

                  {profile?.lookDirection && visibleFilterFields(profile, "advanced").some((f) => f.id === "lookDirection") && (
                    <div className="flex flex-col gap-1" data-condition-field="lookDirection">
                      <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.lookDirection")}</span>
                      <div className="flex items-center gap-1 p-1 bg-sp-surface-muted rounded-sp-sm ring-1 ring-sp-line">
                        {[
                          { label: t("mapWorkbench.conditions.all"), val: "all" },
                          { label: t("mapWorkbench.conditions.lookRight"), val: "RIGHT" },
                          { label: t("mapWorkbench.conditions.lookLeft"), val: "LEFT" },
                        ].map((item) => {
                          const currentLook = lookDirection && lookDirection !== "all" ? lookDirection.toUpperCase() : "all";
                          return (
                            <button
                              key={item.val}
                              type="button"
                              onClick={() => onLookDirectionChange?.(item.val === "all" ? null : item.val)}
                              className={cn(
                                "flex-1 px-1 min-[1600px]:px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                                currentLook === item.val
                                  ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                                  : "text-sp-text-secondary hover:text-sp-text"
                              )}
                            >
                              {item.label}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  )}

                  {profile?.polarization && (() => {
                    const rawAllowed = allowedPolarizationsForBeamMode(profile, beamModes);
                    const dualFirstOrder = ["VV+VH", "HH+HV", "VV", "HH", "VH", "HV"];
                    const orderedAllowed = [
                      ...dualFirstOrder.filter((opt) => rawAllowed.includes(opt)),
                      ...rawAllowed.filter((opt) => !dualFirstOrder.includes(opt)),
                    ];
                    const selectedPols = polarizations || [];
                    const hasSinglePolWithoutDual =
                      profile.selector === "SENTINEL-1" &&
                      selectedPols.some((p) => p === "VV" || p === "VH") &&
                      !selectedPols.includes("VV+VH");
                    const S = WORKBENCH_LOCALE_TEXT[locale].sar;
                    return (
                      <div className="flex flex-col gap-1.5" role="group" aria-label={S.polLabel}>
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-xs text-sp-text-secondary">{S.polLabel}</span>
                          {profile.selector === "ALOS-2" && (beamModes || []).includes("WBS") ? (
                            <span className="text-xs text-sp-text-muted">{t("mapWorkbench.conditions.wbsSinglePolHint")}</span>
                          ) : selectedPols.length > 0 ? (
                            <button
                              type="button"
                              onClick={() => onPolarizationsChange?.([])}
                              className="text-xs text-sp-text-muted hover:text-sp-text cursor-pointer"
                            >
                              {S.polClearDefault}
                            </button>
                          ) : null}
                        </div>
                        <div className="flex flex-wrap gap-1.5">
                          <button
                            type="button"
                            onClick={() => onPolarizationsChange?.([])}
                            className={cn(
                              "h-[26px] px-2.5 rounded-sp-sm text-xs transition-colors cursor-pointer select-none inline-flex items-center justify-center border",
                              selectedPols.length === 0
                                ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                : "border-sp-line text-sp-text-secondary bg-sp-card hover:text-sp-text hover:bg-sp-hover"
                            )}
                          >
                            {t("mapWorkbench.conditions.all")}
                          </button>
                          {orderedAllowed.map((opt) => {
                            const isSelected = selectedPols.includes(opt);
                            return (
                              <button
                                key={opt}
                                type="button"
                                onClick={() => {
                                  if (!profile.polarization?.multiple) {
                                    onPolarizationsChange?.(isSelected ? [] : [opt]);
                                    return;
                                  }
                                  const next = isSelected
                                    ? selectedPols.filter((p) => p !== opt)
                                    : [...selectedPols, opt];
                                  onPolarizationsChange?.(next);
                                }}
                                className={cn(
                                  "h-[26px] px-2.5 rounded-sp-sm text-xs transition-colors cursor-pointer select-none inline-flex items-center justify-center border",
                                  isSelected
                                    ? "border-sp-accent-fill bg-sp-accent-fill text-white font-semibold"
                                    : "border-sp-line text-sp-text-secondary bg-sp-card hover:text-sp-text hover:bg-sp-hover"
                                )}
                              >
                                {opt}
                              </button>
                            );
                          })}
                        </div>
                        {hasSinglePolWithoutDual && (
                          <div className="flex items-center justify-between gap-2 rounded-sp-sm bg-sp-surface-muted px-2 py-1 text-xs text-sp-text-secondary">
                            <span>{S.iwDualPolHint}</span>
                            <button
                              type="button"
                              onClick={() =>
                                onPolarizationsChange?.(
                                  Array.from(new Set([...selectedPols, "VV+VH"]))
                                )
                              }
                              className="shrink-0 font-semibold text-sp-text hover:underline underline-offset-2 cursor-pointer"
                            >
                              {S.includeVvVhBtn}
                            </button>
                          </div>
                        )}
                      </div>
                    );
                  })()}
                </div>
              )}
            </div>
            )}
          </>
        )}

        {/* ================= 模式 2: DEM 规划 (A3b) ================= */}
        {scientificMode === "dem" && (() => {
          const rawSpec = DEM_DATASET_SPECS[demSource] || DEM_DATASET_SPECS.COP30;
          const D = WORKBENCH_LOCALE_TEXT[locale].demTable;
          const specDatum =
            demSource === "FABDEM"
              ? D.datumEgm2008Bare
              : demSource === "GEM15_DTM"
                ? D.datumEgm96Bare
                : demSource === "GEM15_DSM"
                  ? D.datumEgm96Surface
                  : demSource === "NISAR_DEM" || demSource.endsWith("_ELLIPSOIDAL")
                    ? D.datumWgs84Ellipsoid
                    : demSource.startsWith("COP")
                      ? D.datumEgm2008
                      : D.datumEgm96;
          const specChannel =
            demSource.startsWith("COP")
              ? D.channelAws
              : demSource === "FABDEM"
                ? D.channelFabdem
                : demSource === "GEM15_DTM" || demSource === "GEM15_DSM"
                  ? D.channelGem15
                  : demSource === "NISAR_DEM"
                    ? D.channelAsf
                    : demSource === "ASTER_GDEM_V3"
                      ? D.channelEarthdata
                      : D.channelOt;
          const spec = { ...rawSpec, datum: specDatum, channel: specChannel };
          const demSummary = toolOptions.lastDemPlanSummary;
          return (
            <>
              {/* DEM 模式同样保留数据源目录入口，便于用户切换回 SAR / 光学 */}
              <div
                ref={(node) => { groupRefs.current.source = node; }}
                data-condition-group="source"
                data-condition-field="source"
                tabIndex={-1}
                className="bg-sp-surface rounded-[10px] ring-1 ring-sp-line shrink-0 overflow-visible focus:outline-none"
              >
                <button
                  type="button"
                  aria-expanded={groupState.source}
                  aria-controls="group-source-body-dem"
                  onClick={() => handleToggle("source")}
                  className="w-full h-9 flex items-center gap-2 px-3 text-xs font-semibold text-sp-text cursor-pointer select-none text-left"
                >
                  {groupState.source ? (
                    <ChevronDown className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                  ) : (
                    <ChevronRight className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                  )}
                  <span className="shrink-0">{t("mapWorkbench.conditions.sourceAndSatellite")}</span>
                  <span className="ml-auto text-xs font-normal text-sp-text-muted truncate min-w-0" title={activeSourceCardTitle}>
                    {activeSourceCardTitle}
                  </span>
                </button>
                {groupState.source && (
                  <div id="group-source-body-dem" className="p-3 pt-0">
                    {renderDataSourceCard()}
                  </div>
                )}
              </div>

              {/* DEM 规划复用空间范围 AOI */}
              {renderAoiGroup()}

              {/* DEM 数据源与高程基准参数 */}
              <div className="bg-sp-surface rounded-sp-md ring-1 ring-sp-line shrink-0 overflow-hidden">
                <div className="h-9 flex items-center justify-between gap-1.5 px-2.5 text-xs font-semibold text-sp-text select-none min-w-0">
                  <div className="flex items-center gap-1.5 min-w-0">
                    <Layers className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                    <span className="whitespace-nowrap truncate">{t("mapWorkbench.conditions.demParameters")}</span>
                  </div>
                  <span className="rounded-full bg-sp-surface-muted px-2 py-0.5 text-xs font-normal text-sp-text-secondary whitespace-nowrap shrink-0">
                    {spec.resolution}
                  </span>
                </div>
                <div className="p-3 pt-0 flex flex-col gap-3">
                  <div className="flex flex-col gap-1">
                    <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.demSource")}</span>
                    <SpSelect
                      value={demSource}
                      onChange={(val) => onDemSourceChange?.(val)}
                      options={DEM_DATASET_OPTIONS.filter((opt) => opt.enabled).map((opt) => ({
                        value: opt.value,
                        label: opt.label,
                      }))}
                      aria-label={t("mapWorkbench.conditions.demSource")}
                    />
                    <div className="mt-1 grid grid-cols-2 gap-1.5 rounded-sp-sm bg-sp-surface-muted p-2 text-xs text-sp-text-secondary">
                      <div>
                        <span className="text-sp-text-muted">{D.sourceDatumLabel}</span>
                        <span className="font-medium text-sp-text">{spec.datum}</span>
                      </div>
                      <div>
                        <span className="text-sp-text-muted">{D.geoidModelLabel}</span>
                        <span className="font-medium text-sp-text">{spec.geoid}</span>
                      </div>
                      <div className="col-span-2 flex flex-wrap items-baseline gap-x-1 min-w-0">
                        <span className="text-sp-text-muted whitespace-nowrap">{D.channelLabel}</span>
                        <span className="font-medium text-sp-text whitespace-nowrap">{spec.channel}</span>
                      </div>
                    </div>

                    {demSource === "FABDEM" && (
                      <div
                        data-testid="fabdem-noncommercial-license-banner"
                        role="note"
                        className="mt-1.5 flex items-start gap-2 rounded-sp-sm border border-sp-line bg-sp-warning-bg p-2.5 text-xs text-sp-text"
                      >
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-sp-warning-text" />
                        <div className="flex flex-col gap-0.5 leading-snug">
                          <span className="font-semibold text-sp-warning-text">
                            {t("mapWorkbench.conditions.fabdemLicenseTitle")}
                          </span>
                          <span className="text-sp-text-secondary">
                            {t("mapWorkbench.conditions.fabdemLicenseBody")}
                          </span>
                        </div>
                      </div>
                    )}

                    {(demSource === "GEM15_DTM" || demSource === "GEM15_DSM") && (
                      <div
                        data-testid="gem15-coverage-notice-banner"
                        role="note"
                        className="mt-1.5 flex items-start gap-2 rounded-sp-sm border border-sp-line bg-sp-warning-bg p-2.5 text-xs text-sp-text"
                      >
                        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-sp-warning-text" />
                        <div className="flex flex-col gap-0.5 leading-snug">
                          <span className="font-semibold text-sp-warning-text">
                            {t("mapWorkbench.conditions.gem15CoverageTitle")}
                          </span>
                          <span className="text-sp-text-secondary">
                            {t("mapWorkbench.conditions.gem15CoverageBody")}
                          </span>
                        </div>
                      </div>
                    )}

                    {/* Copernicus DEM 多源提供商分段选择器 (Strategy C) */}
                    {!hideMultiProviderSelector && (demSource === "COP30" || isMultiProviderDataset(demSource)) && (() => {
                      const cfg = localizeMultiProviderConfig(
                        MULTI_PROVIDER_CONFIGS[demSource] || MULTI_PROVIDER_CONFIGS["COP30"],
                        locale,
                      );
                      const activeProvider = getActiveDatasetProvider(demSource);
                      return (
                        <div className="flex flex-col gap-1.5 pt-1" data-testid="multi-provider-selector">
                          <span className="text-xs text-sp-text-secondary font-medium">
                            {cfg.title}
                          </span>
                          <div className="flex flex-col gap-1">
                            {cfg.options.map((opt) => {
                              const isSelected = activeProvider === opt.id;
                              return (
                                <button
                                  key={opt.id}
                                  type="button"
                                  data-testid={`provider-option-${opt.id}`}
                                  aria-checked={isSelected}
                                  role="radio"
                                  onClick={() => setActiveDatasetProvider(demSource, opt.id)}
                                  className={cn(
                                    "w-full px-2.5 py-2 rounded-sp-sm border text-left text-xs transition-colors flex flex-col items-start gap-0.5 cursor-pointer",
                                    isSelected
                                      ? "bg-sp-selected border-sp-text text-sp-text font-medium shadow-xs"
                                      : "bg-sp-surface border-sp-line text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                                  )}
                                >
                                  <span className="flex items-center gap-1.5 w-full min-w-0">
                                    <span
                                      className={cn(
                                        "inline-flex items-center justify-center w-3 h-3 rounded-full border shrink-0",
                                        isSelected
                                          ? "border-sp-text bg-sp-text"
                                          : "border-sp-text-muted bg-transparent"
                                      )}
                                    >
                                      {isSelected && <span className="w-1.5 h-1.5 rounded-full bg-sp-surface" />}
                                    </span>
                                    <span className="truncate">{opt.label}</span>
                                  </span>
                                  {opt.subtitle && (
                                    <span className="pl-[18px] text-xs leading-snug text-sp-text-muted break-words line-clamp-2">
                                      {opt.subtitle}
                                    </span>
                                  )}
                                </button>
                              );
                            })}
                          </div>
                        </div>
                      );
                    })()}
                  </div>

                  <div className="flex flex-col gap-1">
                    <label className="flex items-start gap-2 cursor-pointer select-none rounded-sp-sm border border-sp-line bg-sp-card p-2">
                      <input
                        type="checkbox"
                        checked={autoConvertEllipsoid}
                        onChange={() => onToggleAutoConvertEllipsoid?.()}
                        className="mt-0.5 h-4 w-4 rounded-sp-xs border-sp-line text-sp-accent accent-sp-accent"
                      />
                      <div className="flex flex-col gap-0.5">
                        <span className="text-xs font-medium text-sp-text">{t("mapWorkbench.conditions.autoWgs84")}</span>
                        <span className="text-xs text-sp-text-muted leading-snug">
                          {D.autoConvertHint.replace("{geoid}", spec.geoid)}
                        </span>
                      </div>
                    </label>
                  </div>

                  {/* AOI 瓦片覆盖估算与方案预览 */}
                  <div className="rounded-sp-sm border border-sp-line bg-sp-surface-muted p-2.5 flex flex-col gap-2">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-xs font-medium text-sp-text">
                        {workingAoi
                          ? D.estimatedTilesCount.replace("{count}", String(estimatedDemTileCount))
                          : D.bindAoiFirst}
                      </span>
                      <button
                        type="button"
                        disabled={!workingAoi || toolPreviewBusy}
                        onClick={() => void handlePreviewDemPlan()}
                        className="h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer"
                      >
                        {toolPreviewBusy ? D.calculating : D.previewPlanBtn}
                      </button>
                    </div>
                    {workingAoi && (
                      <div className="font-mono text-xs text-sp-text-secondary">
                        {D.boundsPrefix}{workingAoi.west.toFixed(3)}°E ~ {workingAoi.east.toFixed(3)}°E, {workingAoi.south.toFixed(3)}°N ~ {workingAoi.north.toFixed(3)}°N
                      </div>
                    )}
                    {demSummary && demSummary.ok && (
                      <div className="border-t border-sp-line pt-1.5 flex flex-col gap-1 text-xs text-sp-text-secondary">
                        <div>
                          {D.datumPlanPrefix}<span className="font-medium text-sp-text">{demSummary.sourceDatum} → {demSummary.targetDatum}</span>
                        </div>
                        {demSummary.rawDemPath && (
                          <div className="truncate" title={demSummary.rawDemPath}>
                            {D.mosaicArtifactPrefix}<span className="font-mono text-sp-text">{demSummary.rawDemPath}</span>
                          </div>
                        )}
                        {demSummary.sarscapeDemPath && (
                          <div className="truncate" title={demSummary.sarscapeDemPath}>
                            SARscape：<span className="font-mono text-sp-text">{demSummary.sarscapeDemPath}</span>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </div>
            </>
          );
        })()}

        {/* ================= 模式 3: 精密轨道匹配 (A3b) ================= */}
        {scientificMode === "orbit" && (() => {
          const O = WORKBENCH_LOCALE_TEXT[locale].orbitMode;
          return (
          <div className="bg-sp-surface rounded-sp-md ring-1 ring-sp-line shrink-0 overflow-hidden">
            <div className="h-9 flex items-center justify-between gap-2 px-3 text-xs font-semibold text-sp-text select-none">
              <div className="flex items-center gap-2">
                <Orbit className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                <span>{t("mapWorkbench.conditions.preciseOrbitMatch")}</span>
              </div>
              <span className="rounded-full bg-sp-surface-muted px-2 py-0.5 text-xs font-normal text-sp-text-secondary">
                AUX_POEORB / RESORB
              </span>
            </div>
            <div className="p-3 pt-0 flex flex-col gap-3">
              <div className="flex flex-col gap-1">
                <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.matchSource")}</span>
                <div className="flex items-center gap-1 p-1 bg-sp-surface-muted rounded-sp-sm ring-1 ring-sp-line">
                  {[
                    { label: t("mapWorkbench.conditions.currentResults"), val: "current" },
                    { label: t("mapWorkbench.conditions.localSlcDirectory"), val: "local" },
                  ].map((item) => (
                    <button
                      key={item.val}
                      type="button"
                      onClick={() => onOrbitSourceTypeChange?.(item.val as "current" | "local")}
                      className={cn(
                        "flex-1 px-2 py-1 text-xs text-center rounded-sp-sm whitespace-nowrap transition-colors cursor-pointer",
                        orbitSourceType === item.val
                          ? "bg-sp-surface text-sp-text font-semibold shadow-card"
                          : "text-sp-text-secondary hover:text-sp-text"
                      )}
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
              </div>

              {orbitSourceType === "local" ? (
                <div className="flex flex-col gap-2">
                  <div className="flex flex-col gap-1">
                    <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.localSlcImageDirectory")}</span>
                    <div className="flex gap-1.5">
                      <input
                        type="text"
                        placeholder={O.localInputPlaceholder}
                        value={localSlcPath}
                        onChange={(e) => onChangeLocalSlcPath?.(e.target.value)}
                        className="h-8 flex-1 min-w-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none"
                      />
                      <button
                        type="button"
                        onClick={onPickLocalSlcPath}
                        className="h-8 px-2 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
                      >
                        <FolderOpen className="h-3.5 w-3.5" />
                        <span>{t("mapWorkbench.conditions.browse")}</span>
                      </button>
                      <button
                        type="button"
                        onClick={async () => {
                          try {
                            const picked = await pickOpenFile(O.pickScriptTitle, [
                              O.pickScriptFilter,
                            ]);
                            if (picked && picked.ok && picked.path) {
                              onChangeLocalSlcPath?.(picked.path);
                            }
                          } catch {}
                        }}
                        title={O.pickScriptBtnTitle}
                        className="h-8 px-2 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
                      >
                        <FileCode className="h-3.5 w-3.5" />
                        <span>{O.scriptBtn}</span>
                      </button>
                    </div>
                  </div>
                  <div className="rounded-sp-sm bg-sp-surface-muted p-2 text-xs text-sp-text-secondary leading-relaxed">
                    {O.localScanDesc}
                  </div>
                </div>
              ) : (
                <div className="flex flex-col gap-2.5">
                  <div className="p-2.5 rounded-sp-sm bg-sp-surface-muted text-xs text-sp-text-secondary leading-relaxed flex flex-col gap-1">
                    <span className="font-medium text-sp-text">
                      {O.batchCandidateCount.replace("{count}", String(activeSourceView?.item_count ?? 0))}
                    </span>
                    <span className="text-xs">
                      {O.orbitActionHint}
                    </span>
                  </div>
                </div>
              )}
            </div>
          </div>
          );
        })()}

        {/* ================= 模式 4: 高程转换 (A3b) ================= */}
        {scientificMode === "convert" && (() => {
          const C = WORKBENCH_LOCALE_TEXT[locale].convMode;
          const convSummary = toolOptions.lastConvertSummary;
          const resolvedDatum = resolveEffectiveDemSourceDatum(
            localDemPath,
            toolOptions.convertSourceDatum
          );
          const effectiveOutputDir =
            toolOptions.convertOutputDir.trim() || parentDirFromPath(localDemPath);
          return (
            <div className="bg-sp-surface rounded-sp-md ring-1 ring-sp-line shrink-0 overflow-hidden">
              <div className="h-9 flex items-center justify-between gap-2 px-3 text-xs font-semibold text-sp-text select-none">
                <div className="flex items-center gap-2">
                  <Layers className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                  <span>{t("mapWorkbench.conditions.localDemConversion")}</span>
                </div>
                <span className="rounded-full bg-sp-surface-muted px-2 py-0.5 text-xs font-normal text-sp-text-secondary">
                  h = H + N
                </span>
              </div>
              <div className="p-3 pt-0 flex flex-col gap-3">
                {/* 1. 本地 DEM 输入文件 */}
                <div className="flex flex-col gap-1">
                  <span className="text-xs text-sp-text-secondary">{t("mapWorkbench.conditions.localDemFileOrDirectory")}</span>
                  <div className="flex gap-1.5">
                    <input
                      type="text"
                      placeholder={t("mapWorkbench.conditions.chooseLocalDem")}
                      value={localDemPath}
                      onChange={(e) => onChangeLocalDemPath?.(e.target.value)}
                      className="h-8 flex-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none"
                    />
                    <button
                      type="button"
                      onClick={onPickLocalDemPath}
                      className="h-8 px-2.5 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0"
                    >
                      <FolderOpen className="h-3.5 w-3.5" />
                      <span>{t("mapWorkbench.conditions.browse")}</span>
                    </button>
                  </div>
                  <span className="text-xs text-sp-text-muted leading-tight">
                    {C.formatSupportNote}
                  </span>
                </div>

                {/* 2. 源高程基准选择 */}
                <div className="flex flex-col gap-1">
                  <div className="flex items-center justify-between">
                    <span className="text-xs text-sp-text-secondary">{C.sourceDatumLabel}</span>
                    <span className="text-xs text-sp-text-muted">
                      {C.activeDatumPrefix.replace("{datum}", resolvedDatum.datum)}
                    </span>
                  </div>
                  <SpSelect
                    value={toolOptions.convertSourceDatum}
                    onChange={(val) =>
                      updateWorkbenchToolOptions({
                        convertSourceDatum: val as WorkbenchToolOptionsSnapshot["convertSourceDatum"],
                      })
                    }
                    options={[
                      { value: "auto", label: C.autoOption },
                      { value: "EGM2008", label: C.egm2008Option },
                      { value: "EGM96", label: C.egm96Option },
                      { value: "WGS84_ELLIPSOID", label: C.ellipsoidOption },
                    ]}
                    aria-label={C.sourceDatumLabel}
                  />
                </div>

                {/* 3. 输出产物格式选择 */}
                <div className="flex flex-col gap-1">
                  <span className="text-xs text-sp-text-secondary">{C.targetFormatLabel}</span>
                  <div className="grid grid-cols-2 gap-1.5">
                    {[
                      {
                        val: "sarscape" as const,
                        title: C.formatSarscapeTitle,
                        sub: C.formatSarscapeSub,
                      },
                      {
                        val: "ellipsoid" as const,
                        title: C.formatTifTitle,
                        sub: C.formatTifSub,
                      },
                    ].map((item) => {
                      const active = toolOptions.convertOutputMode === item.val;
                      return (
                        <button
                          key={item.val}
                          type="button"
                          onClick={() => updateWorkbenchToolOptions({ convertOutputMode: item.val })}
                          className={cn(
                            "flex flex-col items-start gap-0.5 rounded-sp-sm border p-2 text-left transition-colors cursor-pointer",
                            active
                              ? "border-sp-text bg-sp-selected text-sp-text"
                              : "border-sp-line bg-sp-card text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                          )}
                        >
                          <span className="text-xs font-semibold">{item.title}</span>
                          <span className="text-xs text-sp-text-muted leading-tight">{item.sub}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* 4. 输出目录选择 */}
                <div className="flex flex-col gap-1">
                  <span className="text-xs text-sp-text-secondary">{C.outputDirLabel}</span>
                  <div className="flex gap-1.5">
                    <input
                      type="text"
                      placeholder={effectiveOutputDir || C.outputDirPlaceholder}
                      value={toolOptions.convertOutputDir}
                      onChange={(e) => updateWorkbenchToolOptions({ convertOutputDir: e.target.value })}
                      className="h-8 flex-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text placeholder:text-sp-text-disabled outline-none"
                    />
                    <button
                      type="button"
                      onClick={async () => {
                        try {
                          const picked = await pickDirectory(C.pickDirTitle);
                          if (picked && picked.ok && picked.path) {
                            updateWorkbenchToolOptions({ convertOutputDir: picked.path });
                          }
                        } catch {}
                      }}
                      className="h-8 px-2.5 rounded-sp-sm ring-1 ring-sp-line bg-sp-surface text-xs text-sp-text hover:bg-sp-hover inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0"
                    >
                      <FolderOpen className="h-3.5 w-3.5" />
                      <span>{t("mapWorkbench.conditions.browse")}</span>
                    </button>
                  </div>
                </div>

                {/* 5. 转换方案预览与执行结果卡片 */}
                <div className="rounded-sp-sm border border-sp-line bg-sp-surface-muted p-2.5 flex flex-col gap-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-xs font-semibold text-sp-text">
                      {convSummary?.completed ? C.resultHeading : C.previewHeading}
                    </span>
                    <div className="flex items-center gap-1.5">
                      {localDemPath && (
                        <button
                          type="button"
                          disabled={toolPreviewBusy}
                          onClick={() => void handlePreviewLocalDemConversion()}
                          className="h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer"
                        >
                          {toolPreviewBusy ? C.identifying : C.previewBtn}
                        </button>
                      )}
                      {convSummary?.outputDir && (
                        <span
                          className="h-6 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-mono text-sp-text-secondary inline-flex items-center truncate max-w-[160px]"
                          title={convSummary.outputDir}
                        >
                          {convSummary.completed ? C.writtenBadge : C.readyBadge}
                        </span>
                      )}
                    </div>
                  </div>

                  {convSummary ? (
                    <div className="flex flex-col gap-1 text-xs text-sp-text-secondary">
                      <div>
                        {C.datumConversionPrefix}
                        <span className="font-medium text-sp-text">
                          {convSummary.sourceDatum} → {convSummary.targetDatum}
                          {convSummary.geoidModel
                            ? C.builtinGeoidNote.replace("{model}", convSummary.geoidModel)
                            : ""}
                        </span>
                      </div>
                      {convSummary.ellipsoidDemPath && (
                        <div className="truncate" title={convSummary.ellipsoidDemPath}>
                          {C.ellipsoidTifPrefix}<span className="font-mono text-sp-text">{convSummary.ellipsoidDemPath}</span>
                        </div>
                      )}
                      {convSummary.sarscapeDemPath && toolOptions.convertOutputMode === "sarscape" && (
                        <div className="truncate" title={convSummary.sarscapeDemPath}>
                          SARscape：<span className="font-mono text-sp-text">{convSummary.sarscapeDemPath} (.hdr / .sml)</span>
                        </div>
                      )}
                      {convSummary.message && (
                        <div className="text-xs text-sp-text-muted leading-snug">{convSummary.message}</div>
                      )}
                    </div>
                  ) : (
                    <span className="text-xs text-sp-text-muted leading-relaxed">
                      {t("mapWorkbench.conditions.demConversionDescription")}
                    </span>
                  )}
                </div>
              </div>
            </div>
          );
        })()}

        {/* ================= 模式 5: Google Earth Engine 云端计算与分块下载 ================= */}
        {scientificMode === "gee" && (
          <GeeWorkbenchPanel
            workingAoi={workingAoi}
            aoiLabel={aoiLabel}
            startDate={startDate}
            endDate={endDate}
            onStartDateChange={onStartDateChange}
            onEndDateChange={onEndDateChange}
            onOpenCatalog={onOpenCatalog ? () => onOpenCatalog({ geeOnly: true }) : undefined}
            onSyncGeeResults={onSyncGeeResults}
          />
        )}
      </div>

      {/* 底部检索按钮与内联结果摘要（EOStage Concentric Shell 检索动作栏） */}
      {scientificMode !== "gee" && (
      <div className="p-2.5 border-t border-sp-line bg-[#fafafa] dark:bg-[#141416] flex flex-col gap-2 shrink-0">
        {/* SAR 模式显示检索错误 (Item 1) */}
        {scientificMode === "sar" && searchError && (
          <div className="text-xs text-sp-danger-text leading-tight px-1 break-words">
            {searchError}
          </div>
        )}

        {/* 单行内联结果 (A3c) */}
        {inlineToolResult && (
          <div
            className={cn(
              "flex items-center justify-between gap-1 px-2 py-1 rounded-sp-sm text-xs leading-snug ring-1",
              inlineToolResult.ok
                ? "bg-sp-surface text-sp-success-text ring-sp-line"
                : "bg-sp-danger-bg text-sp-danger-text ring-sp-line"
            )}
          >
            <span className="truncate flex-1" title={inlineToolResult.message}>
              {inlineToolResult.message}
            </span>
            {onDismissToolResult && (
              <button
                type="button"
                onClick={onDismissToolResult}
                aria-label={t("mapWorkbench.conditions.dismissNotice")}
                className="h-4 w-4 shrink-0 inline-flex items-center justify-center text-sp-text-muted hover:text-sp-text rounded-sp-sm cursor-pointer"
              >
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
        )}

        {(() => {
          const B = WORKBENCH_LOCALE_TEXT[locale].bottomSummary;
          return (
        <div className="eo-concentric-band">
          <div className="flex items-center justify-between px-2.5 py-1 text-xs text-[#1e293b] dark:text-[#bfdbfe]">
            <span className="truncate font-medium">
              {activeResourceId === "orbit.sentinel-1.poeorb.v1" && effectiveOrbitSubMode === "local"
                ? (localSlcPath
                    ? B.localSlcFolder.replace("{name}", localSlcPath.split(/[/\\]/).pop() || localSlcPath)
                    : B.chooseLocalSlc)
                : workingAoi
                  ? B.restrictedAoi.replace("{aoi}", aoiLabel || B.customAoiName)
                  : B.globalOrView}
            </span>
            <span
              className="font-mono text-xs opacity-80 shrink-0 ml-2 max-w-[120px] truncate"
              title={
                activeResourceId === "auxiliary.xyz-tiles.v1" && selectedXyzZooms.length > 0
                  ? `Z${selectedXyzZooms.join(",")}`
                  : undefined
              }
            >
              {activeResourceId === "auxiliary.xyz-tiles.v1"
                ? (selectedXyzZooms.length > 0 ? `Z${selectedXyzZooms.join(",")}` : "")
                : activeResourceId === "atmospheric.gacos.ztd.v1"
                ? B.gacosSummary
                    .replace("{dates}", String(gacosAudit.validDates.length))
                    .replace("{h}", String(gacosAudit.utcHourNum).padStart(2, "0"))
                    .replace("{m}", String(gacosAudit.utcMinuteNum).padStart(2, "0"))
                    .replace("{fmt}", toolOptions.gacosOutputFormat === "binary" ? "ZTD" : "GeoTIFF")
                : scientificMode === "dem"
                ? (workingAoi
                    ? B.approxDemTiles.replace("{count}", String(estimatedDemTileCount))
                    : "")
                : scientificMode === "convert"
                ? ""
                : (lastSearchText && scientificMode === "sar" ? lastSearchText : timeInfo)}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              data-testid="btn-start-search"
              onClick={onTriggerSearch}
              disabled={isSearchDisabled}
              aria-busy={searching}
              title={
                searching
                  ? t("mapWorkbench.conditions.processing")
                  : validationErrors.length > 0
                  ? validationErrors.map((e) => e.message).join("；")
                  : undefined
              }
              className="eo-tactile-primary flex-1 min-w-0 h-8 rounded-[10px] bg-[#18181b] text-white dark:bg-white dark:text-[#18181b] text-xs font-semibold disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center justify-center gap-1.5 cursor-pointer whitespace-nowrap"
            >
              {showSearchSpinner ? (
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
              ) : (
                <Search className="h-3.5 w-3.5 shrink-0" />
              )}
              <span>
                {showSearchElapsed
                  ? B.elapsedSeconds
                      .replace("{label}", searchButtonLabel)
                      .replace("{sec}", searchElapsedSeconds.toFixed(1))
                  : searchButtonLabel}
              </span>
              {conditionsChanged && !searching && scientificMode === "sar" && (
                <span className="text-xs font-normal opacity-80 before:content-['·_']">{t("mapWorkbench.conditions.changed")}</span>
              )}
            </button>
            {searching && onCancelSearch && (
              <button
                type="button"
                data-testid="btn-cancel-search"
                onClick={onCancelSearch}
                disabled={cancellingSearch}
                className="h-8 shrink-0 rounded-[10px] border border-sp-danger-text/30 bg-sp-danger-bg px-2.5 text-xs font-semibold text-sp-danger-text hover:opacity-90 disabled:opacity-50 inline-flex items-center gap-1 cursor-pointer whitespace-nowrap"
              >
                <Square className="h-3 w-3 shrink-0 fill-current" />
                <span>{cancellingSearch ? B.cancelling : B.abort}</span>
              </button>
            )}
          </div>
        </div>
          );
        })()}
      </div>
      )}

      <VectorFeaturePickerModal
        open={Boolean(pendingVectorPreview)}
        preview={pendingVectorPreview}
        busy={vectorBusy}
        onCancel={() => setPendingVectorPreview(null)}
        onConfirm={async ({ featureIds, displayField, displayName }) => {
          if (pendingVectorPreview) {
            await finalizeVectorFeatures(pendingVectorPreview, featureIds, displayField, displayName);
          }
        }}
      />
    </section>
  );
}
