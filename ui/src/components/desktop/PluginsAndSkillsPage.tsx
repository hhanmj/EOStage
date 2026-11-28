/**
 * PluginsAndSkillsPage.tsx
 * 「插件与技能」一级主功能路由页面（EOStage 地球科学技能包与可扩展工具中心）
 * 图标统一采用项目唯一的 lucide-react 线性图标库（strokeWidth=1.75，继承文字 currentColor），
 * 配色严格遵循 EOStage 中性单色设计令牌（--sp-surface / --sp-card / --sp-text / --sp-text-secondary / --sp-line）。
 */

import React, {
  useState,
  useEffect,
  useMemo,
  useCallback,
  useRef,
} from "react";
import {
  Puzzle,
  Cable,
  Package,
  Search,
  Plus,
  Check,
  Edit2,
  Trash2,
  RefreshCw,
  Loader2,
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Clock,
  FileCode,
  FolderOpen,
  FileText,
  Folder,
  Info,
  X,
} from "lucide-react";
import { useI18n } from "@/i18n";
import {
  BUNDLED_PLUGIN_LOCALIZATION,
  localizeAgentSkillForDisplay,
  localizeSkillAuthor,
} from "@/lib/pluginsLocale";
import { cn } from "@/lib/utils";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { SpSelect } from "@/components/ui/SpSelect";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon, EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import {
  type AgentSkill,
  type SkillDiscipline,
  type SkillFileEntry,
  getAllSkills,
  saveCustomSkills,
  loadCustomSkills,
  exportCustomSkillsJson,
  importCustomSkillsFromJson,
  loadDisabledSkillIds,
  toggleSkillEnabled,
} from "@/lib/agentSkills";
import { syncDisabledPreferencesToBackend } from "@/lib/channelPreferences";
import {
  type RuntimeComponentsReport,
  type ExtensionRegistryReport,
  type ExtensionRegistryEntry,
  type PluginsStatusResponse,
  type PluginPackageEntry,
  EXTENSION_MIRROR_CHANGED_EVENT,
  normalizeExtensionMirrorSelection,
  loadPersistedExtensionMirror,
  savePersistedExtensionMirror,
  isTauriDesktopHost,
  pickDirectory,
  getRuntimeComponents,
  fetchExtensionRegistry,
  installRuntimeComponent,
  getPluginsStatus,
  installPluginPackage,
  setPluginEnabledBackend,
  uninstallPluginPackageBackend,
  saveCustomConnectorBackend,
  deleteCustomConnectorBackend,
  probeCustomConnectorToolsBackend,
  type McpConnectorProbeResult,
  saveCustomSkillBackend,
  deleteCustomSkillBackend,
  installCustomSkillFromUrlBackend,
  installCustomSkillFromFolderBackend,
  installCondaGeoPackageBackend,
  type ExternalGisInspectReport,
  type ExternalGisPrefs,
  type ExternalGisToolchainInfo,
  inspectExternalGis,
  saveExternalGisPrefs,
} from "@/lib/bridge";
import { Popconfirm } from "@/components/ui/Popconfirm";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { completeLongWork, failLongWork, startLongWork } from "@/lib/longWorkRegistry";

const NON_USER_CUSTOM_AUTHORS = new Set([
  "",
  "EOStage",
  "User Custom",
  "Local Import",
  "Local Folder",
  "远程导入",
]);

function resolveSkillAuthorDisplay(skill: AgentSkill): string {
  const rawAuthor = (skill.author || "").trim();
  if (!skill.isBuiltin) {
    if (NON_USER_CUSTOM_AUTHORS.has(rawAuthor)) {
      return "User";
    }
    return rawAuthor;
  }
  return rawAuthor || "EOStage";
}

function isThirdPartyBuiltinSkill(skill: AgentSkill): boolean {
  const author = (skill.author || "").trim();
  return Boolean(skill.isBuiltin && author && author !== "EOStage");
}

function isHubSkillInstalled(
  entry: ExtensionRegistryEntry,
  currentSkills: AgentSkill[],
): boolean {
  const targetId = entry.skill?.id || `hub-${entry.id}`;
  const bareId = entry.id.replace(/^skill-/, "");
  return currentSkills.some(
    (s) =>
      s.id === targetId ||
      s.id === entry.id ||
      s.id === bareId ||
      s.name === entry.name,
  );
}

const RUNTIME_COMPONENT_META: Record<
  string,
  {
    underDevelopment: boolean;
    sortOrder: number;
    capabilitySummary: string;
  }
> = {
  "eostage-core": {
    underDevelopment: false,
    sortOrder: 0,
    capabilitySummary:
      "提供软件界面、多数据源检索下载调度与智能工作台核心能力，随主程序内置无需额外安装。",
  },
  "native-gis-runtime": {
    underDevelopment: false,
    sortOrder: 1,
    capabilitySummary:
      "本地栅格重投影、高程基准转换、光学影像裁剪镶嵌与标准格式导出。",
  },
  "vertical-grid-egm96": {
    underDevelopment: false,
    sortOrder: 2,
    capabilitySummary:
      "EGM96 正高 DEM 与 WGS84 椭球高之间的高程基准转换。",
  },
  "vertical-grid-egm2008": {
    underDevelopment: false,
    sortOrder: 3,
    capabilitySummary:
      "2.5 分高精度 EGM2008 大地水准面转换，适用于 InSAR 与精密地形对齐。",
  },
  "compat-worker": {
    underDevelopment: false,
    sortOrder: 4,
    capabilitySummary:
      "Windows 专用兼容隔离进程，用于 SARscape 交付与特定矢量/高程格式桥接。",
  },
  "whitebox-cli": {
    underDevelopment: true,
    sortOrder: 10,
    capabilitySummary:
      "地形填洼、河网提取与流域水文分析。",
  },
  "duckdb-cli": {
    underDevelopment: true,
    sortOrder: 11,
    capabilitySummary:
      "Overture Maps 等大规模 GeoParquet 矢量数据高速查询。",
  },
  "rustgis-cli": {
    underDevelopment: true,
    sortOrder: 12,
    capabilitySummary:
      "轻量离线矢量叠加分析与本地空间计算。",
  },
};

const EXTERNAL_GIS_CAPABILITY_SUMMARY: Record<string, string> = {
  qgis: "高级地图制图导出、矢量拓扑与 PyQGIS 本地分析。",
  geolibre: "开源遥感与空间自动化脚本处理。",
  rustgis: "通过 MCP 协议调用本地轻量几何计算与文件检查工具。",
  arcgis_pro: "仅支持 Windows；调用本机 ArcPy 空间分析工具箱与工程环境。",
};

export type PluginsTabKey = "skills" | "tools" | "plugins";

export interface PluginsAndSkillsPageProps {
  initialTab?: PluginsTabKey;
  onNavigate?: (route: any) => void;
  onOpenRuntimeSettings?: () => void;
  surface?: "workspace" | "settings";
  fetchRegistryOverride?: (registryUrl?: string) => Promise<ExtensionRegistryReport>;
  getPluginsStatusOverride?: () => Promise<PluginsStatusResponse | null>;
}

export interface RuntimeToolsSettingsSectionProps {
  fetchRegistryOverride?: (registryUrl?: string) => Promise<ExtensionRegistryReport>;
  getRuntimeComponentsOverride?: () => Promise<RuntimeComponentsReport>;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "--";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1048576).toFixed(2)} MB`;
}

interface CustomConnectorItem {
  id: string;
  name: string;
  transport: "http" | "stdio";
  endpoint: string;
  description: string;
  enabled: boolean;
}

const CUSTOM_CONNECTORS_STORAGE_KEY = "eostage.agent.customConnectors.v1";
const INSTALLED_PLUGINS_STORAGE_KEY = "eostage.agent.installedPluginIds.v1";
const CUSTOM_PLUGINS_STORAGE_KEY = "eostage.agent.customPlugins.v1";
const DISABLED_PLUGINS_STORAGE_KEY = "eostage.agent.disabledPluginIds.v1";

function loadCustomConnectors(): CustomConnectorItem[] {
  if (typeof window === "undefined" || !window.localStorage) return [];
  try {
    const raw = window.localStorage.getItem(CUSTOM_CONNECTORS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function saveCustomConnectors(items: CustomConnectorItem[]): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      CUSTOM_CONNECTORS_STORAGE_KEY,
      JSON.stringify(items),
    );
  } catch {
    // ignore
  }
}

const getDisciplineFilters = (t: (k: any) => string): Array<{
  id: "all" | SkillDiscipline | "builtin" | "custom";
  label: string;
}> => [
  { id: "all", label: t("shell.pluginsPage.categoryAll") },
  { id: "earth-env", label: t("shell.pluginsPage.categoryEarthEnv") },
  { id: "gis-spatial", label: t("shell.pluginsPage.categoryGisSpatial") },
  { id: "insar-sar", label: t("shell.pluginsPage.categoryInsarSar") },
  { id: "skill-engineering", label: t("shell.pluginsPage.categorySkillEngineering") },
  { id: "custom", label: t("shell.pluginsPage.categoryCustom") },
];

export interface PluginSuiteItem {
  id: string;
  name: string;
  version: string;
  author: "EOStage" | string;
  license?: string;
  description: string;
  discipline: SkillDiscipline;
  skillIds?: string[];
  connectorIds?: string[];
  tags?: string[];
  isOfficial?: boolean;
  installPath?: string;
  sha256?: string;
  fileCount?: number;
  bundledSkills?: Array<{
    id: string;
    name: string;
    description: string;
    prompt: string;
  }>;
  bundledConnector?: {
    id: string;
    name: string;
    transport: "http" | "stdio";
    endpoint: string;
    description: string;
  };
  bundledConnectors?: Array<{
    id: string;
    name: string;
    transport: "http" | "stdio";
    endpoint: string;
    description: string;
  }>;
  isCustomImport?: boolean;
  isRetired?: boolean;
}

const DEFAULT_INSTALLED_PLUGIN_IDS: string[] = [];

export const RETIRED_MARKETPLACE_PLUGIN_IDS = new Set<string>([
  "eostage-era5-geohazard-plugin",
  "eostage-grace-hydro-plugin",
  "eostage-pyqgis-cartography-plugin",
  "eostage-reproducible-provenance-plugin",
  "eostage-sbas-insar-plugin",
  "eostage-stac-harmonized-plugin",
  "eostage-whitebox-duckdb-plugin",
  "eostage-eo-recipes-plugin",
  "eostage-gee-cookbook-plugin",
  "eostage-gee-practice-plugin",
  "eostage-eo-methodology-plugin",
]);

export const MARKETPLACE_PLUGIN_SUITES: PluginSuiteItem[] = [
  {
    id: "eostage-gee-plugin",
    name: "Google Earth Engine 科学目录与云端计算插件",
    version: "1.0.0",
    author: "EOStage",
    license: "GPL-3.0-or-later",
    description:
      "集成 Google Earth Engine (GEE) 公共遥感目录检索、数据集选型、按 AOI 云端导出预演、云端分析、分块安全导出与配额状态诊断原生工具集。",
    discipline: "earth-env",
    tags: ["Google Earth Engine", "GEE 云端计算", "AOI 在线裁剪", "原生工具集"],
    bundledSkills: [
      {
        id: "gee-assistant",
        name: "Google Earth Engine (GEE) 遥感科学检索与云端计算规程",
        description:
          "按 EOStage 原生 GEE 工具链执行目录检索、可用性核查、导出预演、云端分析与分块 GeoTIFF 导出",
        prompt:
          "# Google Earth Engine Catalog & Cloud Compute Workflow (`gee-assistant`)\n1. Call `gee_quota_status` first to verify GEE OAuth / Cloud Project readiness;\n2. Use `gee_catalog_search` / `gee_dataset_advisor` and `gee_check_availability` to verify scene counts over the active AOI;\n3. Execute `gee_cloud_analyze` or `gee_script_inspect`, then verify dimensions with `gee_export_preflight` before running `gee_chunked_download`.",
      },
    ],
  },
  {
    id: "eostage-gdal-stream-plugin",
    name: "GDAL /vsicurl/ 在线 COG 流式处理与地形/指数计算插件",
    version: "1.0.0",
    author: "EOStage",
    license: "GPL-3.0-or-later",
    description:
      "基于本地 Native GIS (GDAL 3.12 + PROJ 9.7) 的 /vsicurl/ HTTP Range 流式算子套件，支持免全量下载直接在线裁剪远程 STAC/COG 并计算坡度、坡向、山体阴影、TPI/TRI、粗糙度与 NDVI/NDWI/NBR。",
    discipline: "gis-spatial",
    tags: ["GDAL /vsicurl/", "COG 在线流式处理", "地形算子", "光谱指数"],
    bundledSkills: [
      {
        id: "plugin-skill-gdal-vsicurl-stream",
        name: "GDAL /vsicurl/ 云优化 GeoTIFF (COG) 在线流式窗口处理规程",
        description:
          "免下载全图，通过 HTTP Range 直接读取远程 STAC/COG 指定 BBox 窗口并完成地形因子或植被水体指数计算",
        prompt:
          "# GDAL /vsicurl/ 在线流式窗口处理规程\n1. 优先调用 gis_raster_inspect 检查远程 COG 的 CRS、分辨率、波段数与 NoData；\n2. 调用 gis_raster_process 传入 crop_bbox=[west,south,east,north] 与 operation(clip/slope/aspect/hillshade/tpi/tri/roughness/ndvi/ndwi/nbr)；\n3. 核验输出 GeoTIFF 的有效像元占比与统计极值。",
      },
    ],
  },
  {
    id: "eostage-open-geodata-plugin",
    name: "开放地学数据、多镜像 DEM 与国内坐标互转套件",
    version: "1.0.0",
    author: "EOStage",
    license: "GPL-3.0-or-later",
    description:
      "整合免密钥全球 DEM 多镜像解析 (COP30/COP90/NASADEM/AW3D30)、中国行政区划与标准地名检索、DataV 行政边界、Open-Meteo ERA5 历史气象、Overpass OSM 路网水系、WorldPop 人口网格及 WGS84/GCJ02/BD09/CGCS2000 互转。",
    discipline: "earth-env",
    tags: ["Open GeoData", "多镜像 DEM", "GCJ02/BD09 坐标转换", "ERA5/OSM"],
    bundledSkills: [
      {
        id: "plugin-skill-open-geodata-fusion",
        name: "多源开放地学数据自动获取与国内坐标配准规程",
        description:
          "一键获取研究区行政边界、多镜像 DEM 瓦片、ERA5 历史降水与 OSM 矢量，并自动校正 GCJ-02/BD-09 偏移至 WGS84/CGCS2000",
        prompt:
          "# 开放地学数据获取与坐标配准规程\n1. 使用 open_geodata_query(dataset=mca_xzqh/datav_bound) 获取目标行政区编码与 GeoJSON 边界；\n2. 使用 open_geodata_query(dataset=open_dem_tiles) 解析 AWS/OpenTopography/Planetary Computer 多镜像 COG 链接；\n3. 若叠加国内互联网地图坐标，调用 lbs_coord_transform 统一转换至 WGS84/CGCS2000。",
      },
    ],
  },
  {
    id: "eostage-mapgis-compat-plugin",
    name: "MapGIS 6.x 点线面 (.wt/.wl/.wp) 拓扑解析与 Shapefile/GeoJSON 转换插件",
    version: "1.0.0",
    author: "EOStage",
    license: "GPL-3.0-or-later",
    description:
      "面向国内地质调查、矿产勘查与地质灾害历史图件，提供原生 MapGIS 6.x 二进制 (WMAP D21/D22/D23) 与明文交换格式 (.wt/.wl/.wp) 拓扑弧段组装并无损导出为 ESRI Shapefile (.shp) 或 GeoJSON。",
    discipline: "gis-spatial",
    tags: ["MapGIS 6.x", ".wt/.wl/.wp", "拓扑造区", "Shapefile 转换"],
    bundledSkills: [
      {
        id: "plugin-skill-mapgis-legacy-migration",
        name: "地质矿产历史图件 MapGIS (.wt/.wl/.wp) 拓扑转换与质检规程",
        description:
          "自动探查 MapGIS 点/线/面文件头、组装有向弧段外环与内岛拓扑，并批量输出标准 Shapefile 或 GeoJSON",
        prompt:
          "# MapGIS 6.x 矢量图件转换规程\n1. 调用 mapgis_inspect 检查 .wt/.wl/.wp 是二进制格式还是明文交换格式，核实要素数量与坐标量级；\n2. 若为毫米图面坐标，设置适当的 scale 缩放系数；\n3. 调用 mapgis_convert 导出为 .shp 或 .geojson 并验证闭合环拓扑。",
      },
    ],
  },
];

function loadInstalledPluginIds(): Set<string> {
  if (typeof window === "undefined" || !window.localStorage) {
    return new Set(DEFAULT_INSTALLED_PLUGIN_IDS);
  }
  try {
    const raw = window.localStorage.getItem(INSTALLED_PLUGINS_STORAGE_KEY);
    if (!raw) return new Set(DEFAULT_INSTALLED_PLUGIN_IDS);
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((x) => typeof x === "string"))
      : new Set(DEFAULT_INSTALLED_PLUGIN_IDS);
  } catch {
    return new Set(DEFAULT_INSTALLED_PLUGIN_IDS);
  }
}

function saveInstalledPluginIds(ids: Iterable<string>): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      INSTALLED_PLUGINS_STORAGE_KEY,
      JSON.stringify(Array.from(ids)),
    );
  } catch {
    // ignore
  }
}

function loadCustomPlugins(): PluginSuiteItem[] {
  if (typeof window === "undefined" || !window.localStorage) return [];
  try {
    const raw = window.localStorage.getItem(CUSTOM_PLUGINS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function saveCustomPlugins(items: PluginSuiteItem[]): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      CUSTOM_PLUGINS_STORAGE_KEY,
      JSON.stringify(items),
    );
  } catch {
    // ignore
  }
}

function loadDisabledPluginIds(): Set<string> {
  if (typeof window === "undefined" || !window.localStorage) return new Set();
  try {
    const raw = window.localStorage.getItem(DISABLED_PLUGINS_STORAGE_KEY);
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((x) => typeof x === "string"))
      : new Set();
  } catch {
    return new Set();
  }
}

function saveDisabledPluginIds(ids: Iterable<string>): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    window.localStorage.setItem(
      DISABLED_PLUGINS_STORAGE_KEY,
      JSON.stringify(Array.from(ids)),
    );
  } catch {
    // ignore
  }
}

const NATIVE_REMOTE_SENSING_SKILL_IDS = new Set([
  "dem-datum-conversion",
  "s1-precise-orbit",
  "nisar-rslc-subset",
  "s1-tops-burst",
  "gacos-ztd-atmo",
]);

export function PluginsAndSkillsPage({
  initialTab,
  onNavigate: _onNavigate,
  onOpenRuntimeSettings,
  surface,
  fetchRegistryOverride,
  getPluginsStatusOverride,
}: PluginsAndSkillsPageProps) {
  const { t, locale } = useI18n();
  const hasLiveRegistryHost = Boolean(fetchRegistryOverride) || isTauriDesktopHost();
  const [embeddedInSettings, setEmbeddedInSettings] = useState(
    surface === "settings",
  );
  const isSettingsSurface = surface === "settings" || embeddedInSettings;
  const [activeTab, setActiveTab] = useState<PluginsTabKey>(() => {
    if (initialTab === "skills" || initialTab === "tools" || initialTab === "plugins") {
      return initialTab;
    }
    return surface === "settings" ? "tools" : "skills";
  });
  const rootRef = useRef<HTMLDivElement | null>(null);
  const mainRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (surface !== undefined) return;
    if (rootRef.current?.closest(".eo-settings-card")) {
      setEmbeddedInSettings(true);
      if (initialTab === undefined) {
        setActiveTab("tools");
      }
    }
  }, [surface, initialTab]);

  useEffect(() => {
    if (mainRef.current && typeof mainRef.current.scrollTo === "function") {
      mainRef.current.scrollTo({ top: 0 });
    }
  }, [activeTab]);

  // ----------------------------------------------------
  // 扩展仓库 (eostage-hub) 注册表状态
  // ----------------------------------------------------
  const [registryUrlInput, setRegistryUrlInput] = useState<string>(
    () => loadPersistedExtensionMirror().url,
  );
  const registryUrlInputRef = useRef<string>(registryUrlInput);
  registryUrlInputRef.current = registryUrlInput;
  const [registryReport, setRegistryReport] =
    useState<ExtensionRegistryReport | null>(null);
  const [registryLoading, setRegistryLoading] = useState(false);
  const [registryError, setRegistryError] = useState<string>("");

  const { spinClass: registrySpinClass } = useFullTurnSpin(registryLoading);

  const handleLoadRegistry = useCallback(
    async (overrideUrl?: string, manual = false) => {
      const latestPersisted = loadPersistedExtensionMirror().url;
      if (overrideUrl === undefined && latestPersisted !== registryUrlInputRef.current) {
        registryUrlInputRef.current = latestPersisted;
        setRegistryUrlInput(latestPersisted);
      }
      const targetUrl =
        overrideUrl !== undefined
          ? overrideUrl
          : latestPersisted || registryUrlInputRef.current.trim();
      const fetcher = fetchRegistryOverride ?? fetchExtensionRegistry;
      setRegistryLoading(true);
      setRegistryError("");
      const lwId = manual
        ? startLongWork({
            title: t("shell.pluginsPage.syncMarketTitle"),
            category: "plugin",
            stageLabel: t("shell.pluginsPage.syncMarketStage"),
          })
        : null;
      try {
        const report = await fetcher(targetUrl ? targetUrl : undefined);
        setRegistryReport(report);
        if (manual) {
          emitTopToast({ text: t("shell.pluginsPage.syncMarketToast"), tone: "success" });
          if (lwId) {
            completeLongWork(lwId, {
              doneTitle: t("shell.pluginsPage.syncMarketToast"),
              doneSubtitle: t("shell.pluginsPage.syncMarketSubtitle", {
                count: report?.entries?.length ?? 0,
              }),
              onOpen: () => setActiveTab("plugins"),
            });
          }
        }
      } catch (err: any) {
        setRegistryReport(null);
        const msg =
          err?.context || err?.message || String(err || t("shell.pluginsPage.syncMarketFailed"));
        setRegistryError(msg);
        if (lwId) {
          failLongWork(lwId, msg);
        }
      } finally {
        setRegistryLoading(false);
      }
    },
    [registryUrlInput, fetchRegistryOverride],
  );
  const handleLoadRegistryRef = useRef(handleLoadRegistry);
  handleLoadRegistryRef.current = handleLoadRegistry;

  const registryConnectionState: "syncing" | "connected" | "offline-fallback" =
    registryLoading
      ? "syncing"
      : hasLiveRegistryHost &&
          !registryError &&
          registryReport?.ok === true &&
          Boolean((registryReport.source_url || "").trim())
        ? "connected"
        : "offline-fallback";

  const registryAutoLoadedRef = useRef(false);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const syncMirrorFromStorage = () => {
      const nextUrl = loadPersistedExtensionMirror().url;
      if (nextUrl === registryUrlInputRef.current) return;
      registryUrlInputRef.current = nextUrl;
      setRegistryUrlInput(nextUrl);
      if (hasLiveRegistryHost && registryAutoLoadedRef.current) {
        void handleLoadRegistryRef.current(nextUrl, false);
      }
    };
    const handleStorage = (ev: StorageEvent) => {
      if (
        ev.key === null ||
        ev.key === "eostage.runtime.mirrorPreset.v1" ||
        ev.key === "eostage.runtime.registryUrl.v1"
      ) {
        syncMirrorFromStorage();
      }
    };
    window.addEventListener(EXTENSION_MIRROR_CHANGED_EVENT, syncMirrorFromStorage);
    window.addEventListener("storage", handleStorage);
    return () => {
      window.removeEventListener(
        EXTENSION_MIRROR_CHANGED_EVENT,
        syncMirrorFromStorage,
      );
      window.removeEventListener("storage", handleStorage);
    };
  }, [hasLiveRegistryHost]);

  useEffect(() => {
    if (!hasLiveRegistryHost || !isTauriDesktopHost()) return;
    let cancelled = false;
    void inspectExternalGis(false)
      .then((report) => {
        if (cancelled || !report?.prefs) return;
        const { extension_mirror_preset, extension_registry_url } =
          report.prefs;
        if (!extension_mirror_preset && !extension_registry_url) return;
        const normalized = normalizeExtensionMirrorSelection(
          extension_mirror_preset,
          extension_registry_url,
        );
        if (normalized.url !== registryUrlInputRef.current) {
          registryUrlInputRef.current = normalized.url;
          setRegistryUrlInput(normalized.url);
          savePersistedExtensionMirror(normalized.presetId, normalized.url);
          if (registryAutoLoadedRef.current) {
            void handleLoadRegistryRef.current(normalized.url, false);
          }
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [hasLiveRegistryHost]);

  useEffect(() => {
    if ((activeTab === "skills" || activeTab === "plugins") && hasLiveRegistryHost) {
      if (!registryAutoLoadedRef.current && !registryReport && !registryLoading) {
        registryAutoLoadedRef.current = true;
        void handleLoadRegistry();
      }
    }
  }, [
    activeTab,
    hasLiveRegistryHost,
    registryReport,
    registryLoading,
    handleLoadRegistry,
  ]);

  // ----------------------------------------------------
  // 页签 ① 技能 (Skills) 状态
  // ----------------------------------------------------
  const [skills, setSkills] = useState<AgentSkill[]>(getAllSkills);
  const [disabledSkillIds, setDisabledSkillIds] =
    useState<Set<string>>(loadDisabledSkillIds);
  const [skillSearch, setSkillSearch] = useState("");
  const [skillFilter, setSkillFilter] = useState<
    "all" | SkillDiscipline | "builtin" | "custom"
  >("all");
  const [copiedSkillId, setCopiedSkillId] = useState<string | null>(null);
  const [skillsBanner, setSkillsBanner] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);

  // 技能包详情查看器 (Skill Package Inspector)
  const [inspectedSkillId, setInspectedSkillId] = useState<string | null>(null);
  const [inspectorActiveNode, setInspectorActiveNode] =
    useState<string>("__overview__");
  const [dynamicFileContent, setDynamicFileContent] = useState<
    Record<string, string>
  >({});
  const [loadingFilePath, setLoadingFilePath] = useState<string | null>(null);

  const inspectedSkill = useMemo(() => {
    if (!inspectedSkillId) return null;
    const found = skills.find((s) => s.id === inspectedSkillId);
    return found ? localizeAgentSkillForDisplay(found, locale) : null;
  }, [skills, inspectedSkillId, locale]);

  const handleInspectSkill = (skill: AgentSkill) => {
    setInspectedSkillId(skill.id);
    setInspectorActiveNode("__overview__");
  };

  useEffect(() => {
    if (!inspectedSkillId) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setInspectedSkillId(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [inspectedSkillId]);

  const handleSelectInspectorFile = async (
    skill: AgentSkill,
    relPath: string,
  ) => {
    setInspectorActiveNode(relPath);
    if (skill.bundledFileContents?.[relPath] !== undefined) {
      return;
    }
    const cacheKey = `${skill.id}:${relPath}`;
    if (dynamicFileContent[cacheKey] !== undefined) {
      return;
    }
    setLoadingFilePath(relPath);
    try {
      const resp = await fetch(`/skills/${skill.id}/${relPath}`);
      if (resp.ok) {
        const text = await resp.text();
        setDynamicFileContent((prev) => ({ ...prev, [cacheKey]: text }));
      } else {
        setDynamicFileContent((prev) => ({
          ...prev,
          [cacheKey]: t("shell.pluginsPage.binaryFileHint", {
            id: skill.id,
            relPath,
          }),
        }));
      }
    } catch {
      setDynamicFileContent((prev) => ({
        ...prev,
        [cacheKey]: `# ${relPath} (runtime/skills/${skill.id}/${relPath})`,
      }));
    } finally {
      setLoadingFilePath(null);
    }
  };

  const inspectorFileTree = useMemo(() => {
    if (!inspectedSkill)
      return {
        dirs: [] as Array<[string, SkillFileEntry[]]>,
        rootFiles: [] as SkillFileEntry[],
      };
    const files: SkillFileEntry[] = inspectedSkill.files || [
      {
        path: "SKILL.md",
        kind: "skill_md",
        sizeBytes: inspectedSkill.prompt.length,
      },
    ];
    const dirMap = new Map<string, SkillFileEntry[]>();
    const rootFiles: SkillFileEntry[] = [];
    for (const f of files) {
      const slashIdx = f.path.indexOf("/");
      if (slashIdx > 0) {
        const dirName = f.path.slice(0, slashIdx);
        const list = dirMap.get(dirName) || [];
        list.push(f);
        dirMap.set(dirName, list);
      } else {
        rootFiles.push(f);
      }
    }
    return {
      dirs: Array.from(dirMap.entries()),
      rootFiles,
    };
  }, [inspectedSkill]);

  const currentInspectedFileText = useMemo(() => {
    if (!inspectedSkill || inspectorActiveNode === "__overview__") return "";
    if (
      inspectedSkill.bundledFileContents?.[inspectorActiveNode] !== undefined
    ) {
      return inspectedSkill.bundledFileContents[inspectorActiveNode];
    }
    if (inspectorActiveNode === "SKILL.md") {
      return inspectedSkill.prompt;
    }
    const cacheKey = `${inspectedSkill.id}:${inspectorActiveNode}`;
    return dynamicFileContent[cacheKey] || "";
  }, [inspectedSkill, inspectorActiveNode, dynamicFileContent]);

  // 导入/导出与通过链接添加面板状态
  const [importPanelOpen, setImportPanelOpen] = useState(false);
  const [importJsonText, setImportJsonText] = useState("");
  const [addSkillByUrlOpen, setAddSkillByUrlOpen] = useState(false);
  const [skillUrlInput, setSkillUrlInput] = useState("");
  const [addingSkillByUrl, setAddingSkillByUrl] = useState(false);

  // 新建/编辑技能右侧抽屉
  const [skillModalOpen, setSkillModalOpen] = useState(false);
  const [editingSkill, setEditingSkill] = useState<AgentSkill | null>(null);
  const [formName, setFormName] = useState("");
  const [formDesc, setFormDesc] = useState("");
  const [formPrompt, setFormPrompt] = useState("");
  const [formError, setFormError] = useState("");
  const skillDrawerOpenerRef = useRef<HTMLElement | null>(null);
  const skillNameInputRef = useRef<HTMLInputElement | null>(null);
  const skillCloseBtnRef = useRef<HTMLButtonElement | null>(null);

  const [pendingPopconfirm, setPendingPopconfirm] = useState<{
    kind: "skill" | "connector" | "plugin" | "discard-skill-form";
    id: string;
    suite?: PluginSuiteItem;
    anchorEl: HTMLElement | null;
    title: string;
    description: string;
    confirmLabel: string;
  } | null>(null);

  const refreshSkills = useCallback(() => {
    setSkills(getAllSkills());
    setDisabledSkillIds(loadDisabledSkillIds());
  }, []);

  const handleToggleSkillEnabled = (skillId: string) => {
    const current = disabledSkillIds.has(skillId);
    const nextSet = toggleSkillEnabled(skillId, current);
    setDisabledSkillIds(new Set(nextSet));
    void syncDisabledPreferencesToBackend(Array.from(nextSet));
  };

  const hasUnsavedSkillDraft = useMemo(() => {
    if (!skillModalOpen) return false;
    const baseName = editingSkill?.name ?? "";
    const baseDesc = editingSkill?.description ?? "";
    const basePrompt = editingSkill?.prompt ?? "";
    return (
      formName.trim() !== baseName.trim() ||
      formDesc.trim() !== baseDesc.trim() ||
      formPrompt.trim() !== basePrompt.trim()
    );
  }, [skillModalOpen, editingSkill, formName, formDesc, formPrompt]);

  const closeSkillDrawerNow = useCallback(() => {
    setSkillModalOpen(false);
    const opener = skillDrawerOpenerRef.current;
    if (opener && opener.isConnected) {
      setTimeout(() => {
        if (opener.isConnected) opener.focus();
      }, 0);
    }
  }, []);

  const requestCloseSkillDrawer = useCallback(
    (anchorEl?: HTMLElement | null) => {
      if (hasUnsavedSkillDraft) {
        setPendingPopconfirm({
          kind: "discard-skill-form",
          id: editingSkill?.id ?? "new-skill",
          anchorEl: anchorEl ?? skillCloseBtnRef.current,
          title: t("shell.pluginsPage.discardSkillTitle"),
          description: t("shell.pluginsPage.discardSkillDesc"),
          confirmLabel: t("shell.pluginsPage.discardSkillConfirm"),
        });
        return;
      }
      closeSkillDrawerNow();
    },
    [hasUnsavedSkillDraft, editingSkill, closeSkillDrawerNow, t],
  );

  useEffect(() => {
    if (!skillModalOpen) return;
    const timer = setTimeout(() => {
      skillNameInputRef.current?.focus();
    }, 0);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented && !pendingPopconfirm) {
        e.preventDefault();
        requestCloseSkillDrawer(skillCloseBtnRef.current);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [skillModalOpen, pendingPopconfirm, requestCloseSkillDrawer]);

  const handleOpenCreateSkill = (e?: React.MouseEvent<HTMLElement>) => {
    skillDrawerOpenerRef.current =
      e?.currentTarget ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setEditingSkill(null);
    setFormName("");
    setFormDesc("");
    setFormPrompt("");
    setFormError("");
    setSkillModalOpen(true);
  };

  const handleOpenEditSkill = (skill: AgentSkill, e?: React.MouseEvent<HTMLElement>) => {
    if (skill.isBuiltin) return;
    skillDrawerOpenerRef.current =
      e?.currentTarget ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setEditingSkill(skill);
    setFormName(skill.name);
    setFormDesc(skill.description);
    setFormPrompt(skill.prompt);
    setFormError("");
    setSkillModalOpen(true);
  };

  const handleSaveSkill = (e: React.FormEvent) => {
    e.preventDefault();
    const name = formName.trim();
    const desc = formDesc.trim();
    const prompt = formPrompt.trim();

    if (!name) {
      setFormError(t("shell.pluginsPage.nameRequiredErr"));
      return;
    }
    if (!desc) {
      setFormError(t("shell.pluginsPage.descRequiredErr"));
      return;
    }
    if (!prompt) {
      setFormError(t("shell.pluginsPage.promptRequiredErr"));
      return;
    }

    const currentCustom = loadCustomSkills();
    const nowIso = new Date().toISOString();

    if (editingSkill) {
      const normalizedEditAuthor = resolveSkillAuthorDisplay(editingSkill);
      const prevCustom = [...currentCustom];
      const updated = currentCustom.map((item) =>
        item.id === editingSkill.id
          ? {
              ...item,
              name,
              description: desc,
              prompt,
              author: normalizedEditAuthor,
              updatedAt: nowIso,
            }
          : item,
      );
      saveCustomSkills(updated);
      void saveCustomSkillBackend({
        id: editingSkill.id,
        name,
        description: desc,
        prompt,
        author: normalizedEditAuthor,
        license: editingSkill.license || "Custom",
        version: editingSkill.version || "1.0.0",
      }).catch((err: any) => {
        saveCustomSkills(prevCustom);
        refreshSkills();
        setSkillsBanner({
          type: "error",
          text:
            err?.message ||
            t("shell.pluginsPage.saveSkillFailedRollback", { name }),
        });
      });
    } else {
      const prevCustom = [...currentCustom];
      const slug = name
        .toLowerCase()
        .replace(/[^a-z0-9\u4e00-\u9fa5_-]+/g, "-")
        .replace(/^-+|-+$/g, "");
      const newSkill: AgentSkill = {
        id: `custom-skill-${Date.now()}`,
        name,
        description: desc,
        overview: desc,
        prompt,
        isBuiltin: false,
        category: "custom",
        discipline: "custom",
        author: "User",
        license: "Custom",
        version: "1.0.0",
        slashCommand: `/${slug || "custom-skill"}`,
        sourceLabel: t("shell.pluginsPage.categoryCustom"),
        updatedAt: nowIso,
      };
      saveCustomSkills([...currentCustom, newSkill]);
      void saveCustomSkillBackend({
        id: newSkill.id,
        name: newSkill.name,
        description: newSkill.description,
        prompt: newSkill.prompt,
        author: "User",
        license: newSkill.license || "Custom",
        version: newSkill.version || "1.0.0",
      }).catch((err: any) => {
        saveCustomSkills(prevCustom);
        refreshSkills();
        setSkillsBanner({
          type: "error",
          text:
            err?.message ||
            t("shell.pluginsPage.createSkillFailedRollback", { name }),
        });
      });
    }

    closeSkillDrawerNow();
    refreshSkills();
  };

  const performDeleteSkill = (skillId: string) => {
    const currentCustom = loadCustomSkills();
    const prevCustom = [...currentCustom];
    const updated = currentCustom.filter((s) => s.id !== skillId);
    saveCustomSkills(updated);
    void deleteCustomSkillBackend(skillId).catch((err: any) => {
      saveCustomSkills(prevCustom);
      refreshSkills();
      setSkillsBanner({
        type: "error",
        text: err?.message || t("shell.pluginsPage.deleteSkillFailedRollback"),
      });
    });
    if (inspectedSkillId === skillId) {
      setInspectedSkillId(null);
    }
    refreshSkills();
  };

  const handleDeleteSkill = (skillId: string, anchorEl?: HTMLElement | null) => {
    const targetSkill = skills.find((s) => s.id === skillId);
    const label = targetSkill?.name || skillId;
    setPendingPopconfirm({
      kind: "skill",
      id: skillId,
      anchorEl: anchorEl ?? null,
      title: t("shell.pluginsPage.deleteSkillConfirmTitle", { name: label }),
      description: t("shell.pluginsPage.deleteConfirm"),
      confirmLabel: t("shell.pluginsPage.deleteSkillConfirmBtn"),
    });
  };

  const handleExportSkills = async () => {
    const json = exportCustomSkillsJson();
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(json);
      }
    } catch {
      // ignore
    }
    setSkillsBanner({
      type: "success",
      text: t("shell.pluginsPage.exportedSkillsToast", { count: loadCustomSkills().length }),
    });
  };

  const handleLocalSkillFileUpload = async (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      setImportJsonText(text);
    } catch {
      // ignore
    }
  };

  const handleConfirmImportSkills = (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const res = importCustomSkillsFromJson(importJsonText);
      const allCustom = loadCustomSkills();
      for (const sk of allCustom.slice(-Math.max(1, res.importedCount))) {
        void saveCustomSkillBackend({
          id: sk.id,
          name: sk.name,
          description: sk.description,
          prompt: sk.prompt,
          author: resolveSkillAuthorDisplay(sk),
          license: sk.license || "Apache-2.0",
          version: sk.version || "1.0.0",
        }).catch(() => {});
      }
      refreshSkills();
      setImportPanelOpen(false);
      setImportJsonText("");
      setSkillsBanner({
        type: "success",
        text: t("shell.pluginsPage.importedSkillsToast", { count: res.importedCount }),
      });
    } catch (err: any) {
      setSkillsBanner({
        type: "error",
        text: err?.message || t("shell.pluginsPage.importSkillsFailed"),
      });
    }
  };

  const handleAddSkillByUrl = async (e: React.FormEvent) => {
    e.preventDefault();
    const url = skillUrlInput.trim();
    if (!url) {
      setSkillsBanner({ type: "error", text: t("shell.pluginsPage.errorEnterSkillUrl") });
      return;
    }
    setAddingSkillByUrl(true);
    try {
      let importedCount = 0;
      if (isTauriDesktopHost()) {
        const statusResp = await installCustomSkillFromUrlBackend(url);
        const backendSkills = statusResp?.custom_skills ?? [];
        if (!statusResp || backendSkills.length === 0) {
          throw new Error(t("shell.pluginsPage.errorNoBackendSkillRecord"));
        }
        const mergedById = new Map<string, AgentSkill>(
          loadCustomSkills().map((s) => [s.id, s]),
        );
        for (const rec of backendSkills) {
          const slugBase = (rec.id || rec.name || "remote-skill")
            .toLowerCase()
            .replace(/[^a-z0-9_-]+/g, "-")
            .replace(/^-+|-+$/g, "");
          mergedById.set(rec.id, {
            id: rec.id,
            name: rec.name,
            description: rec.description || t("shell.pluginsPage.importedFromUrl", { url }),
            overview: rec.description || t("shell.pluginsPage.importedPackageFromUrl", { url }),
            prompt: rec.prompt,
            isBuiltin: false,
            category: "custom",
            discipline: "custom",
            author: rec.author || t("shell.pluginsPage.remoteImportLabel"),
            license: rec.license || "Apache-2.0",
            version: rec.version || "1.0.0",
            slashCommand: `/${rec.slash_command?.replace(/^\//, "") || slugBase || "remote-skill"}`,
            sourceLabel: t("shell.pluginsPage.addByUrlSourceLabel"),
            updatedAt: rec.updated_at || new Date().toISOString(),
          });
        }
        saveCustomSkills(Array.from(mergedById.values()));
        importedCount = 1;
      } else {
        const resp = await fetch(url);
        if (!resp.ok) {
          throw new Error(t("shell.pluginsPage.errorRemoteHttp", { status: resp.status }));
        }
        const fetchedText = (await resp.text()).trim();
        if (!fetchedText) {
          throw new Error(t("shell.pluginsPage.errorUrlEmpty"));
        }
        if (fetchedText.startsWith("[") || fetchedText.startsWith("{")) {
          const res = importCustomSkillsFromJson(fetchedText);
          importedCount = res.importedCount;
          if (importedCount <= 0) {
            throw new Error(t("shell.pluginsPage.errorInvalidJsonCatalog"));
          }
        } else if (fetchedText.startsWith("---") || fetchedText.startsWith("#")) {
          const nameMatch = fetchedText.match(/^name:\s*(.+)$/m);
          const descMatch = fetchedText.match(/^description:\s*(.+)$/m);
          const segments = url.replace(/\/+$/, "").split("/");
          const rawSlug = (segments[segments.length - 1] || "remote-skill")
            .replace(/\.(md|json)$/i, "")
            .replace(/[^a-zA-Z0-9\u4e00-\u9fa5_-]+/g, "-");
          const skillName =
            nameMatch?.[1]?.trim().replace(/^["']|["']$/g, "") ||
            rawSlug ||
            t("shell.pluginsPage.remoteSkillFallbackName");
          const skillDesc =
            descMatch?.[1]?.trim().replace(/^["']|["']$/g, "") ||
            t("shell.pluginsPage.importedFromUrl", { url });
          const custom = loadCustomSkills();
          const newSkill: AgentSkill = {
            id: `url-skill-${Date.now()}`,
            name: skillName,
            description: skillDesc,
            overview: skillDesc,
            prompt: fetchedText,
            isBuiltin: false,
            category: "custom",
            discipline: "custom",
            author: t("shell.pluginsPage.remoteImportLabel"),
            license: "Apache-2.0",
            version: "1.0.0",
            slashCommand: `/${rawSlug.toLowerCase() || "remote-skill"}`,
            sourceLabel: t("shell.pluginsPage.addByUrlSourceLabel"),
            updatedAt: new Date().toISOString(),
          };
          saveCustomSkills([...custom, newSkill]);
          importedCount = 1;
        } else {
          throw new Error(t("shell.pluginsPage.errorInvalidSkillMdOrJson"));
        }
      }

      refreshSkills();
      setAddSkillByUrlOpen(false);
      setSkillUrlInput("");
      setSkillsBanner({
        type: "success",
        text: t("shell.pluginsPage.toastUrlAdded", { count: importedCount, url }),
      });
    } catch (err: any) {
      setSkillsBanner({
        type: "error",
        text: err?.message || t("shell.pluginsPage.errorUrlAddFailed"),
      });
    } finally {
      setAddingSkillByUrl(false);
    }
  };

  const [importingSkillFolder, setImportingSkillFolder] = useState(false);

  const handleImportSkillFolder = async () => {
    setImportingSkillFolder(true);
    try {
      const picked = await pickDirectory(t("shell.pluginsPage.pickSkillDirTitle"));
      if (!picked?.ok || !picked.path?.trim()) {
        return;
      }
      const statusResp = await installCustomSkillFromFolderBackend(
        picked.path.trim(),
      );
      if (!statusResp) {
        setSkillsBanner({
          type: "error",
          text: t("shell.pluginsPage.folderImportDesktopOnly"),
        });
        return;
      }
      const backendSkills = statusResp.custom_skills ?? [];
      const mergedById = new Map<string, AgentSkill>(
        loadCustomSkills().map((s) => [s.id, s]),
      );
      for (const rec of backendSkills) {
        mergedById.set(rec.id, {
          id: rec.id,
          name: rec.name,
          description: rec.description || t("shell.pluginsPage.importedFolderDesc"),
          overview: rec.description || t("shell.pluginsPage.importedFolderOverview"),
          prompt: rec.prompt,
          isBuiltin: false,
          category: "custom",
          discipline: "custom",
          author: rec.author && !NON_USER_CUSTOM_AUTHORS.has(rec.author) ? rec.author : "User",
          license: rec.license || "MIT",
          version: rec.version || "1.0.0",
          slashCommand: rec.slash_command || `/${rec.id}`,
          sourceLabel: t("shell.pluginsPage.folderImportSourceLabel"),
          updatedAt: rec.updated_at || new Date().toISOString(),
        });
      }
      saveCustomSkills(Array.from(mergedById.values()));
      refreshSkills();
      setImportPanelOpen(false);
      setAddSkillByUrlOpen(false);
      const msg = t("shell.pluginsPage.toastFolderImported", { path: picked.path.trim() });
      setSkillsBanner({ type: "success", text: msg });
      emitTopToast({ text: t("shell.pluginsPage.topToastFolderImported"), tone: "success" });
    } catch (err: any) {
      setSkillsBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.errorFolderImportFailed")),
      });
    } finally {
      setImportingSkillFolder(false);
    }
  };

  const handleInstallSkillFromHub = (entry: ExtensionRegistryEntry) => {
    try {
      const targetId = entry.skill?.id || `hub-${entry.id}`;
      const payload = {
        ...(entry.skill || {
          id: targetId,
          name_zh: entry.name,
          name_en: entry.name,
          category: "custom",
          description_zh: entry.description,
          description_en: entry.description,
          prompt_template: entry.description,
        }),
        author: t("shell.pluginsPage.officialHubAuthor"),
      };
      importCustomSkillsFromJson(JSON.stringify([payload]));
      void saveCustomSkillBackend({
        id: targetId,
        name: entry.name,
        description: entry.description,
        prompt: payload.prompt_template || entry.description,
        author: t("shell.pluginsPage.officialHubAuthor"),
        license: "Apache-2.0",
        version: entry.version || "1.0.0",
      }).catch(() => {});
      refreshSkills();
      setSkillsBanner({
        type: "success",
        text: t("shell.pluginsPage.hubInstalledToast", { name: entry.name }),
      });
    } catch (err: any) {
      setSkillsBanner({
        type: "error",
        text: err?.message || t("shell.pluginsPage.hubInstallFailed"),
      });
    }
  };

  const handleCopyPrompt = async (skill: AgentSkill) => {
    try {
      await navigator.clipboard.writeText(skill.prompt);
      setCopiedSkillId(skill.id);
      setTimeout(() => setCopiedSkillId(null), 2000);
    } catch {
      // ignore
    }
  };

  const handleCopyExampleTask = async (
    skill: AgentSkill,
    taskText: string,
  ) => {
    const fullText = `${skill.slashCommand || ""} ${taskText}`.trim();
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(fullText);
      }
    } catch {
      // ignore
    }
    setSkillsBanner({
      type: "success",
      text: t("shell.pluginsPage.copiedExamplePrompt", { text: fullText }),
    });
  };

  const filteredSkills = useMemo(() => {
    return skills.filter((item) => {
      if (skillFilter === "builtin" && !item.isBuiltin) return false;
      if (skillFilter === "custom" && item.isBuiltin) return false;
      if (
        skillFilter !== "all" &&
        skillFilter !== "builtin" &&
        skillFilter !== "custom"
      ) {
        if (item.discipline !== skillFilter) return false;
      }
      if (!skillSearch.trim()) return true;
      const q = skillSearch.toLowerCase();
      const loc = localizeAgentSkillForDisplay(item, locale);
      return (
        item.name.toLowerCase().includes(q) ||
        loc.name.toLowerCase().includes(q) ||
        (item.nameEn && item.nameEn.toLowerCase().includes(q)) ||
        item.description.toLowerCase().includes(q) ||
        loc.description.toLowerCase().includes(q) ||
        (item.slashCommand && item.slashCommand.toLowerCase().includes(q)) ||
        item.prompt.toLowerCase().includes(q)
      );
    });
  }, [skills, skillFilter, skillSearch, locale]);

  const skillFilterCounts = useMemo(() => {
    const counts: Record<string, number> = {
      all: skills.length,
      builtin: 0,
      custom: 0,
    };
    for (const item of skills) {
      if (item.isBuiltin) counts.builtin += 1;
      else counts.custom += 1;
      if (item.discipline) {
        counts[item.discipline] = (counts[item.discipline] ?? 0) + 1;
      }
    }
    return counts;
  }, [skills]);

  const [inspectedHubSkill, setInspectedHubSkill] =
    useState<ExtensionRegistryEntry | null>(null);

  useEffect(() => {
    if (!inspectedHubSkill) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setInspectedHubSkill(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [inspectedHubSkill]);

  const hubSkillStats = useMemo(() => {
    const allEntries = (registryReport?.entries || []).filter(
      (e) => e.kind === "skill",
    );
    let installedCount = 0;
    for (const entry of allEntries) {
      if (isHubSkillInstalled(entry, skills)) {
        installedCount += 1;
      }
    }
    return {
      total: allEntries.length,
      installed: installedCount,
      uninstalled: allEntries.length - installedCount,
    };
  }, [registryReport, skills]);

  const hubSkillEntries = useMemo(() => {
    const q = skillSearch.trim().toLowerCase();
    const list = (registryReport?.entries || []).filter((e) => {
      if (e.kind !== "skill") return false;
      if (!q) return true;
      return (
        e.name.toLowerCase().includes(q) ||
        e.description.toLowerCase().includes(q) ||
        (e.skill?.name_en || "").toLowerCase().includes(q) ||
        (e.capabilities || []).some((c) => c.toLowerCase().includes(q))
      );
    });
    return [...list].sort((a, b) => {
      const aInstalled = isHubSkillInstalled(a, skills) ? 1 : 0;
      const bInstalled = isHubSkillInstalled(b, skills) ? 1 : 0;
      return aInstalled - bInstalled;
    });
  }, [registryReport, skills, skillSearch]);

  // ----------------------------------------------------
  // 页签 ② 连接器与工具 (Connectors & Tools) 状态
  // ----------------------------------------------------
  const [customConnectors, setCustomConnectors] =
    useState<CustomConnectorItem[]>(loadCustomConnectors);
  const [inspectedConnectorId, setInspectedConnectorId] = useState<string | null>(
    null,
  );
  const [copiedConnectorId, setCopiedConnectorId] = useState<string | null>(
    null,
  );
  const copyConnectorTimerRef = useRef<number | null>(null);
  const inspectedConnector = useMemo(() => {
    if (!inspectedConnectorId) return null;
    return customConnectors.find((c) => c.id === inspectedConnectorId) || null;
  }, [customConnectors, inspectedConnectorId]);

  useEffect(() => {
    return () => {
      if (copyConnectorTimerRef.current !== null) {
        window.clearTimeout(copyConnectorTimerRef.current);
        copyConnectorTimerRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!inspectedConnectorId) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setInspectedConnectorId(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [inspectedConnectorId]);
  const [connectorFormOpen, setConnectorFormOpen] = useState(false);
  const [connName, setConnName] = useState("");
  const [connTransport, setConnTransport] = useState<"http" | "stdio">("http");
  const [connEndpoint, setConnEndpoint] = useState("");
  const [connDesc, setConnDesc] = useState("");

  const handleAddCustomConnector = (e: React.FormEvent) => {
    e.preventDefault();
    if (!connName.trim() || !connEndpoint.trim()) return;
    const nextItem: CustomConnectorItem = {
      id: `custom-mcp-${Date.now()}`,
      name: connName.trim(),
      transport: connTransport,
      endpoint: connEndpoint.trim(),
      description: connDesc.trim() || t("shell.pluginsPage.customMcpDefaultDesc"),
      enabled: true,
    };
    const updated = [...customConnectors, nextItem];
    setCustomConnectors(updated);
    saveCustomConnectors(updated);
    void saveCustomConnectorBackend(nextItem).catch(() => {});
    setConnName("");
    setConnEndpoint("");
    setConnDesc("");
    setConnectorFormOpen(false);
  };

  const performDeleteCustomConnector = (id: string) => {
    const updated = customConnectors.filter((c) => c.id !== id);
    setCustomConnectors(updated);
    saveCustomConnectors(updated);
    if (inspectedConnectorId === id) {
      setInspectedConnectorId(null);
    }
    void deleteCustomConnectorBackend(id).catch(() => {});
  };

  const handleDeleteCustomConnector = (id: string, anchorEl?: HTMLElement | null) => {
    const target = customConnectors.find((c) => c.id === id);
    const label = target?.name || id;
    setPendingPopconfirm({
      kind: "connector",
      id,
      anchorEl: anchorEl ?? null,
      title: t("shell.pluginsPage.removeConnectorTitle", { name: label }),
      description: t("shell.pluginsPage.removeConnectorDesc"),
      confirmLabel: t("shell.pluginsPage.removeConnectorConfirm"),
    });
  };

  const handleToggleCustomConnectorEnabled = (id: string) => {
    const updated = customConnectors.map((c) =>
      c.id === id ? { ...c, enabled: !c.enabled } : c,
    );
    setCustomConnectors(updated);
    saveCustomConnectors(updated);
    const target = updated.find((c) => c.id === id);
    if (target) {
      void saveCustomConnectorBackend(target).catch(() => {});
    }
  };

  const handleCopyConnectorEndpoint = (cc: CustomConnectorItem) => {
    if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
      void navigator.clipboard.writeText(cc.endpoint).catch(() => {});
    }
    if (copyConnectorTimerRef.current !== null) {
      window.clearTimeout(copyConnectorTimerRef.current);
      copyConnectorTimerRef.current = null;
    }
    setCopiedConnectorId(cc.id);
    copyConnectorTimerRef.current = window.setTimeout(() => {
      copyConnectorTimerRef.current = null;
      setCopiedConnectorId((prev) => (prev === cc.id ? null : prev));
    }, 1500);
  };

  const [probingConnectorId, setProbingConnectorId] = useState<string | null>(
    null,
  );
  const [connectorProbeById, setConnectorProbeById] = useState<
    Record<string, McpConnectorProbeResult>
  >({});

  const handleProbeConnectorTools = async (connectorId: string) => {
    setProbingConnectorId(connectorId);
    try {
      const res = await probeCustomConnectorToolsBackend(connectorId);
      if (res) {
        setConnectorProbeById((prev) => ({ ...prev, [connectorId]: res }));
      } else {
        setConnectorProbeById((prev) => ({
          ...prev,
          [connectorId]: {
            ok: false,
            connector_id: connectorId,
            transport: "unknown",
            tools: [],
            error: t("shell.pluginsPage.mcpDesktopOnlyErr"),
          },
        }));
      }
    } catch (err: any) {
      setConnectorProbeById((prev) => ({
        ...prev,
        [connectorId]: {
          ok: false,
          connector_id: connectorId,
          transport: "unknown",
          tools: [],
          error:
            err?.context ||
            err?.message ||
            String(err || t("shell.pluginsPage.probeConnectorFailed")),
        },
      }));
    } finally {
      setProbingConnectorId(null);
    }
  };

  // ----------------------------------------------------
  // 页签 ③ 插件 (Plugins) 状态：真实连接 Rust 后端磁盘持久化
  // ----------------------------------------------------
  const [pluginFilter, setPluginFilter] = useState<"added" | "browse">("added");
  const [installedPluginIds, setInstalledPluginIds] = useState<Set<string>>(
    loadInstalledPluginIds,
  );
  const [customPlugins, setCustomPlugins] =
    useState<PluginSuiteItem[]>(loadCustomPlugins);
  const [disabledPluginIds, setDisabledPluginIds] = useState<Set<string>>(
    loadDisabledPluginIds,
  );
  const [backendPluginMeta, setBackendPluginMeta] = useState<
    Record<string, PluginPackageEntry>
  >({});
  const [backendPluginsRoot, setBackendPluginsRoot] = useState<string>("");
  const [pluginSearch, setPluginSearch] = useState("");
  const [pluginAuthorFilter, setPluginAuthorFilter] = useState<string>("all");
  const [pluginImportModalOpen, setPluginImportModalOpen] = useState(false);
  const [pluginImportName, setPluginImportName] = useState("");
  const [pluginImportPath, setPluginImportPath] = useState("");
  const [pluginImportText, setPluginImportText] = useState("");
  const [previewPluginSuite, setPreviewPluginSuite] =
    useState<PluginSuiteItem | null>(null);
  const [pluginsBanner, setPluginsBanner] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);

  useEffect(() => {
    if (!previewPluginSuite) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreviewPluginSuite(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [previewPluginSuite]);

  const syncPluginsFromBackend = useCallback(async () => {
    try {
      const fetchStatus = getPluginsStatusOverride ?? getPluginsStatus;
      const status: PluginsStatusResponse | null = await fetchStatus();
      if (!status) return;
      setBackendPluginsRoot(status.plugins_dir || "");

      const metaMap: Record<string, PluginPackageEntry> = {};
      for (const pkg of status.available_plugins || []) {
        if (!RETIRED_MARKETPLACE_PLUGIN_IDS.has(pkg.id)) {
          metaMap[pkg.id] = pkg;
        }
      }
      // Keep retired plugins when already installed so users can inspect/toggle/uninstall them
      for (const pkg of status.installed_plugins || []) {
        metaMap[pkg.id] = pkg;
      }
      setBackendPluginMeta(metaMap);

      const installedSet = new Set<string>(
        (status.installed_plugins || []).map((p: PluginPackageEntry) => p.id),
      );
      setInstalledPluginIds(installedSet);
      saveInstalledPluginIds(installedSet);

      const disabledSet = new Set<string>(
        (status.installed_plugins || [])
          .filter((p: PluginPackageEntry) => !p.enabled)
          .map((p: PluginPackageEntry) => p.id),
      );
      setDisabledPluginIds(disabledSet);
      saveDisabledPluginIds(disabledSet);

      const builtinIds = new Set(MARKETPLACE_PLUGIN_SUITES.map((m) => m.id));
      const installedCustomEntries: PluginPackageEntry[] = [];
      const seenBackendIds = new Set<string>();
      for (const p of status.installed_plugins || []) {
        if (!seenBackendIds.has(p.id) && !builtinIds.has(p.id)) {
          seenBackendIds.add(p.id);
          installedCustomEntries.push(p);
        }
      }
      const diskCustomSuites: PluginSuiteItem[] = installedCustomEntries.map((p: PluginPackageEntry) => {
        const mappedConnectors =
          p.bundled_connectors && p.bundled_connectors.length > 0
            ? p.bundled_connectors.map((c) => ({
                id: c.id,
                name: c.name,
                transport: (c.transport === "stdio" ? "stdio" : "http") as "http" | "stdio",
                endpoint: c.endpoint,
                description: c.description,
              }))
            : p.bundled_connector
              ? [
                  {
                    id: p.bundled_connector.id,
                    name: p.bundled_connector.name,
                    transport: (p.bundled_connector.transport === "stdio" ? "stdio" : "http") as
                      | "http"
                      | "stdio",
                    endpoint: p.bundled_connector.endpoint,
                    description: p.bundled_connector.description,
                  },
                ]
              : undefined;
        return {
          id: p.id,
          name: p.name,
          version: p.version,
          author: p.author || "EOStage",
          license: p.license || "Apache-2.0",
          description: p.description,
          discipline: (p.discipline as SkillDiscipline) || "custom",
          tags:
            p.tags && p.tags.length > 0
              ? p.tags
              : [t("shell.pluginsPage.importLocalTag")],
          installPath: p.install_path,
          sha256: p.sha256,
          fileCount: p.files?.length || 0,
          bundledSkills: (p.bundled_skills || []).map((sk) => ({
            id: sk.id,
            name: sk.name,
            description: sk.description,
            prompt: sk.prompt,
          })),
          bundledConnector: mappedConnectors?.[0],
          bundledConnectors: mappedConnectors,
          isOfficial: false,
          isCustomImport: true,
          isRetired: RETIRED_MARKETPLACE_PLUGIN_IDS.has(p.id),
        };
      });
      setCustomPlugins(diskCustomSuites);
      saveCustomPlugins(diskCustomSuites);

      if (Array.isArray(status.custom_connectors)) {
        const mappedConns: CustomConnectorItem[] = status.custom_connectors.map(
          (c) => ({
            id: c.id,
            name: c.name,
            transport: c.transport === "stdio" ? "stdio" : "http",
            endpoint: c.endpoint,
            description: c.description,
            enabled: c.enabled,
          }),
        );
        setCustomConnectors(mappedConns);
        saveCustomConnectors(mappedConns);
      }

      if (Array.isArray(status.custom_skills)) {
        const mappedCustomSkills: AgentSkill[] = status.custom_skills.map((sk) => ({
          id: sk.id,
          name: sk.name,
          description: sk.description,
          overview: sk.description,
          prompt: sk.prompt,
          isBuiltin: false,
          category: "custom",
          discipline: "custom",
          author: sk.author || "EOStage",
          license: sk.license || "Apache-2.0",
          version: sk.version || "1.0.0",
          slashCommand: sk.slash_command || `/${sk.id}`,
          sourceLabel:
            sk.author && sk.author.toLowerCase() === "user"
              ? t("shell.pluginsPage.categoryCustom")
              : t("shell.pluginsPage.pluginSourceTag"),
          updatedAt: sk.updated_at || new Date().toISOString(),
        }));
        if (isTauriDesktopHost()) {
          saveCustomSkills(mappedCustomSkills);
          refreshSkills();
        } else if (mappedCustomSkills.length > 0) {
          const localSkills = loadCustomSkills();
          const byId = new Map<string, AgentSkill>(localSkills.map((s) => [s.id, s]));
          let changed = false;
          for (const item of mappedCustomSkills) {
            if (!byId.has(item.id)) {
              byId.set(item.id, item);
              changed = true;
            }
          }
          if (changed) {
            saveCustomSkills(Array.from(byId.values()));
            refreshSkills();
          }
        }
      }
    } catch {
      // keep local state if backend unavailable
    }
  }, [refreshSkills, getPluginsStatusOverride]);

  useEffect(() => {
    void syncPluginsFromBackend();
  }, [syncPluginsFromBackend]);

  const localizePluginSuite = useCallback(
    (suite: PluginSuiteItem): PluginSuiteItem => {
      if (locale === "zh-CN") return suite;
      const loc = BUNDLED_PLUGIN_LOCALIZATION[suite.id]?.[locale];
      if (!loc) return suite;
      return {
        ...suite,
        name: loc.name,
        description: loc.description,
        tags: loc.tags,
        bundledSkills: suite.bundledSkills?.map((b, idx) =>
          idx === 0 && loc.skillName
            ? {
                ...b,
                name: loc.skillName,
                description: loc.skillDescription ?? b.description,
              }
            : b,
        ),
        bundledConnector:
          suite.bundledConnector && loc.connectorDescription
            ? {
                ...suite.bundledConnector,
                description: loc.connectorDescription,
              }
            : suite.bundledConnector,
      };
    },
    [locale],
  );

  const allAvailablePlugins = useMemo(() => {
    const merged = [...MARKETPLACE_PLUGIN_SUITES];
    const seenIds = new Set(merged.map((m) => m.id));
    for (const c of customPlugins) {
      if (!seenIds.has(c.id)) {
        merged.push(c);
        seenIds.add(c.id);
      }
    }
    return merged.map((rawSuite) => {
      const suite = localizePluginSuite(rawSuite);
      const meta = backendPluginMeta[suite.id];
      const isRetired = suite.isRetired || RETIRED_MARKETPLACE_PLUGIN_IDS.has(suite.id);
      if (!meta) {
        return isRetired ? { ...suite, isRetired: true } : suite;
      }
      const metaConnectors =
        meta.bundled_connectors && meta.bundled_connectors.length > 0
          ? meta.bundled_connectors.map((c) => ({
              id: c.id,
              name: c.name,
              transport: (c.transport === "stdio" ? "stdio" : "http") as "http" | "stdio",
              endpoint: c.endpoint,
              description: c.description,
            }))
          : suite.bundledConnectors;
      return {
        ...suite,
        installPath: meta.install_path || suite.installPath,
        sha256: meta.sha256 || suite.sha256,
        fileCount: meta.files?.length ?? suite.fileCount,
        bundledConnectors: metaConnectors,
        bundledConnector: metaConnectors?.[0] ?? suite.bundledConnector,
        isRetired,
      };
    });
  }, [customPlugins, backendPluginMeta, localizePluginSuite]);

  const browseBasePlugins = useMemo(() => {
    return allAvailablePlugins.filter(
      (suite) => !suite.isRetired && !RETIRED_MARKETPLACE_PLUGIN_IDS.has(suite.id),
    );
  }, [allAvailablePlugins]);

  const addedPlugins = useMemo(() => {
    return allAvailablePlugins.filter((suite) => {
      if (!installedPluginIds.has(suite.id)) return false;
      if (
        pluginAuthorFilter !== "all" &&
        suite.author !== pluginAuthorFilter
      ) {
        return false;
      }
      if (!pluginSearch.trim()) return true;
      const q = pluginSearch.toLowerCase();
      return (
        suite.name.toLowerCase().includes(q) ||
        suite.description.toLowerCase().includes(q) ||
        suite.author.toLowerCase().includes(q)
      );
    });
  }, [allAvailablePlugins, installedPluginIds, pluginAuthorFilter, pluginSearch]);

  const browsePlugins = useMemo(() => {
    return browseBasePlugins.filter((suite) => {
      if (
        pluginAuthorFilter !== "all" &&
        suite.author !== pluginAuthorFilter
      ) {
        return false;
      }
      if (!pluginSearch.trim()) return true;
      const q = pluginSearch.toLowerCase();
      return (
        suite.name.toLowerCase().includes(q) ||
        suite.description.toLowerCase().includes(q) ||
        suite.author.toLowerCase().includes(q) ||
        (suite.tags || []).some((tag) => tag.toLowerCase().includes(q))
      );
    });
  }, [browseBasePlugins, pluginAuthorFilter, pluginSearch]);

  const totalAddedPluginCount = useMemo(() => {
    return allAvailablePlugins.filter((s) => installedPluginIds.has(s.id))
      .length;
  }, [allAvailablePlugins, installedPluginIds]);

  const getSuiteSkills = useCallback(
    (suite: PluginSuiteItem) => {
      if (suite.bundledSkills && suite.bundledSkills.length > 0) {
        const bundledIds = new Set(suite.bundledSkills.map((b) => b.id));
        const matched = skills.filter((s) => bundledIds.has(s.id));
        if (matched.length > 0) {
          return matched.map((s) => localizeAgentSkillForDisplay(s, locale));
        }
        return suite.bundledSkills.map((b) =>
          localizeAgentSkillForDisplay(
            {
              id: b.id,
              name: b.name,
              description: b.description,
              overview: b.description,
              prompt: b.prompt,
              isBuiltin: false,
              category: "custom" as const,
              discipline: suite.discipline,
              author: suite.author,
              license: suite.license || "Apache-2.0",
              version: suite.version,
              slashCommand: `/${b.id}`,
              sourceLabel: t("shell.pluginsPage.pluginSourceTag"),
              updatedAt: new Date().toISOString(),
            },
            locale,
          ),
        );
      }
      if (suite.skillIds && suite.skillIds.length > 0) {
        const idSet = new Set(suite.skillIds);
        const matched = skills.filter((s) => idSet.has(s.id));
        if (matched.length > 0) {
          return matched.map((s) => localizeAgentSkillForDisplay(s, locale));
        }
      }
      if (suite.id === "eostage-earth-environment-suite") {
        return skills
          .filter(
            (s) => s.isBuiltin && s.badge === "官方" && s.id !== "skill-creator",
          )
          .map((s) => localizeAgentSkillForDisplay(s, locale));
      }
      if (suite.id === "eostage-insar-native-suite") {
        return skills
          .filter((s) => NATIVE_REMOTE_SENSING_SKILL_IDS.has(s.id))
          .map((s) => localizeAgentSkillForDisplay(s, locale));
      }
      if (Array.isArray(suite.bundledSkills) || suite.isRetired) {
        return [];
      }
      return skills
        .filter((s) => s.id === "skill-creator")
        .map((s) => localizeAgentSkillForDisplay(s, locale));
    },
    [skills, locale, t],
  );

  const handleTogglePluginSuite = (suite: PluginSuiteItem) => {
    const suiteSkills = getSuiteSkills(suite);
    const isCurrentlyDisabled = disabledPluginIds.has(suite.id);
    const prevDisabledSkills = loadDisabledSkillIds();
    const isDesktop = isTauriDesktopHost();
    setDisabledPluginIds((prev) => {
      const next = new Set(prev);
      if (isCurrentlyDisabled) next.delete(suite.id);
      else next.add(suite.id);
      if (!isDesktop) {
        saveDisabledPluginIds(next);
      }
      return next;
    });
    if (!isDesktop) {
      for (const sk of suiteSkills) {
        toggleSkillEnabled(sk.id, isCurrentlyDisabled);
      }
      setDisabledSkillIds(loadDisabledSkillIds());
    }
    const rollbackToggle = (msg?: string) => {
      setDisabledPluginIds((prev) => {
        const reverted = new Set(prev);
        if (isCurrentlyDisabled) reverted.add(suite.id);
        else reverted.delete(suite.id);
        if (!isDesktop) {
          saveDisabledPluginIds(reverted);
        }
        return reverted;
      });
      if (!isDesktop) {
        for (const sk of suiteSkills) {
          toggleSkillEnabled(sk.id, !prevDisabledSkills.has(sk.id));
        }
        setDisabledSkillIds(loadDisabledSkillIds());
      }
      setPluginsBanner({
        type: "error",
        text:
          msg ||
          t("shell.pluginsPage.togglePluginFailedRollback", { name: suite.name }),
      });
    };
    void setPluginEnabledBackend(suite.id, isCurrentlyDisabled)
      .then((res) => {
        if (isDesktop && res && !res.ok) {
          rollbackToggle();
          return;
        }
        return syncPluginsFromBackend();
      })
      .catch((err: any) => {
        rollbackToggle(err?.message);
      });
  };

  const handleInstallPlugin = (suite: PluginSuiteItem) => {
    const isDesktop = isTauriDesktopHost();
    if (!isDesktop && suite.bundledSkills && suite.bundledSkills.length > 0) {
      const currentCustom = loadCustomSkills();
      const existingIds = new Set(currentCustom.map((s) => s.id));
      const newSkills: AgentSkill[] = [];
      for (const b of suite.bundledSkills) {
        if (!existingIds.has(b.id)) {
          newSkills.push({
            id: b.id,
            name: b.name,
            description: b.description,
            overview: b.description,
            prompt: b.prompt,
            isBuiltin: false,
            category: "custom",
            discipline: suite.discipline,
            author: suite.author,
            license: suite.license || "Apache-2.0",
            version: suite.version,
            slashCommand: `/${b.id}`,
            sourceLabel: t("shell.pluginsPage.pluginSourceTag"),
            updatedAt: new Date().toISOString(),
          });
        }
      }
      if (newSkills.length > 0) {
        saveCustomSkills([...currentCustom, ...newSkills]);
        refreshSkills();
      }
    }
    if (!isDesktop && suite.bundledConnector) {
      const currentConns = loadCustomConnectors();
      if (!currentConns.some((c) => c.id === suite.bundledConnector!.id)) {
        const nextConns: CustomConnectorItem[] = [
          ...currentConns,
          {
            ...suite.bundledConnector,
            enabled: true,
          },
        ];
        setCustomConnectors(nextConns);
        saveCustomConnectors(nextConns);
      }
    }
    const suiteSkills = getSuiteSkills(suite);
    if (!isDesktop) {
      setInstalledPluginIds((prev) => {
        const next = new Set(prev);
        next.add(suite.id);
        saveInstalledPluginIds(next);
        return next;
      });
      setDisabledPluginIds((prev) => {
        const next = new Set(prev);
        next.delete(suite.id);
        saveDisabledPluginIds(next);
        return next;
      });
      for (const sk of suiteSkills) {
        toggleSkillEnabled(sk.id, true);
      }
      setDisabledSkillIds(loadDisabledSkillIds());
      setPluginsBanner({
        type: "success",
        text: t("shell.pluginsPage.addPluginSuccess", {
          name: suite.name,
          author: localizeSkillAuthor(suite.author, locale),
          count: suiteSkills.length,
        }),
      });
    }

    void installPluginPackage({ plugin_id: suite.id })
      .then((res) => {
        const installed = res?.installed_plugin;
        if (installed) {
          setBackendPluginMeta((prev) => ({
            ...prev,
            [installed.id]: installed,
          }));
          setPluginsBanner({
            type: "success",
            text: t("shell.pluginsPage.installPluginSuccess", {
              name: suite.name,
              author: localizeSkillAuthor(installed.author, locale),
              path: installed.install_path,
              sha: installed.sha256.slice(0, 12),
            }),
          });
          void syncPluginsFromBackend();
        }
      })
      .catch((err: any) => {
        if (!isDesktop) {
          setInstalledPluginIds((prev) => {
            const next = new Set(prev);
            next.delete(suite.id);
            saveInstalledPluginIds(next);
            return next;
          });
        }
        setPluginsBanner({
          type: "error",
          text:
            err?.message ||
            t("shell.pluginsPage.installPluginFailedRollback", { name: suite.name }),
        });
      });
  };

  const performRemovePlugin = (suite: PluginSuiteItem) => {
    const isDesktop = isTauriDesktopHost();
    const successText =
      suite.isCustomImport || suite.isOfficial === false
        ? t("shell.pluginsPage.removedLocalPlugin", { name: suite.name })
        : t("shell.pluginsPage.removedCataloguePlugin", { name: suite.name });

    if (!isDesktop) {
      setInstalledPluginIds((prev) => {
        const next = new Set(prev);
        next.delete(suite.id);
        saveInstalledPluginIds(next);
        return next;
      });
      if (suite.isCustomImport || suite.isOfficial === false) {
        setCustomPlugins((prev) => {
          const nextCustom = prev.filter((c) => c.id !== suite.id);
          saveCustomPlugins(nextCustom);
          return nextCustom;
        });
      }
      setPluginsBanner({ type: "success", text: successText });
      void uninstallPluginPackageBackend(suite.id)
        .then(() => syncPluginsFromBackend())
        .catch(() => {});
      return;
    }

    const wasCustom = suite.isCustomImport || suite.isOfficial === false;
    const prevInstalled = new Set(installedPluginIds);
    const prevDisabled = new Set(disabledPluginIds);
    const prevCustom = customPlugins;

    setInstalledPluginIds((prev) => {
      const next = new Set(prev);
      next.delete(suite.id);
      return next;
    });
    setDisabledPluginIds((prev) => {
      const next = new Set(prev);
      next.delete(suite.id);
      return next;
    });
    if (wasCustom) {
      setCustomPlugins((prev) => prev.filter((c) => c.id !== suite.id));
    }

    const rollbackRemove = (errMsg?: string) => {
      setInstalledPluginIds(prevInstalled);
      setDisabledPluginIds(prevDisabled);
      if (wasCustom) {
        setCustomPlugins(prevCustom);
      }
      setPluginsBanner({
        type: "error",
        text:
          errMsg ||
          t("shell.pluginsPage.removePluginFailedRollback", { name: suite.name }),
      });
    };

    void uninstallPluginPackageBackend(suite.id)
      .then((res) => {
        if (!res || !res.ok) {
          rollbackRemove();
          return;
        }
        setPluginsBanner({ type: "success", text: successText });
        return syncPluginsFromBackend();
      })
      .catch((err: any) => {
        rollbackRemove(err?.message);
      });
  };

  const handleRemovePlugin = (suite: PluginSuiteItem, anchorEl?: HTMLElement | null) => {
    if (suite.isCustomImport || suite.isOfficial === false) {
      setPendingPopconfirm({
        kind: "plugin",
        id: suite.id,
        suite,
        anchorEl: anchorEl ?? null,
        title: t("shell.pluginsPage.removePluginTitle", { name: suite.name }),
        description: t("shell.pluginsPage.removePluginDesc"),
        confirmLabel: t("shell.pluginsPage.removePluginConfirm"),
      });
      return;
    }
    performRemovePlugin(suite);
  };

  const handleLocalPluginFileUpload = async (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const files = Array.from(e.target.files ?? []);
    if (files.length === 0) return;
    try {
      const manifestFile =
        files.find((f) => /plugin\.json$/i.test(f.name)) ??
        files.find((f) => /skill\.md$/i.test(f.name)) ??
        files.find((f) => /mcp\.json$/i.test(f.name)) ??
        files[0];
      if (!manifestFile) return;
      const nativePath = (manifestFile as File & { path?: string }).path;
      if (nativePath && /\.zip$/i.test(manifestFile.name)) {
        setPluginImportPath(nativePath);
      }
      if (/\.zip$/i.test(manifestFile.name)) {
        const baseName = manifestFile.name.replace(/\.zip$/i, "");
        if (!pluginImportName.trim()) {
          setPluginImportName(baseName);
        }
        setPluginImportText(
          JSON.stringify(
            {
              id: `zip-plugin-${baseName.toLowerCase().replace(/[^a-z0-9-_]+/g, "-")}`,
              name: baseName,
              version: "1.0.0",
              author: "EOStage",
              description: t("shell.pluginsPage.pluginLocalDesc", { name: manifestFile.name }),
            },
            null,
            2,
          ),
        );
        return;
      }
      const text = await manifestFile.text();
      setPluginImportText(text);
      if (!pluginImportName.trim()) {
        const relPath = (manifestFile as File & { webkitRelativePath?: string })
          .webkitRelativePath;
        const folderName = relPath ? relPath.split("/")[0] : "";
        setPluginImportName(
          folderName || manifestFile.name.replace(/\.(json|md|yaml|yml|zip)$/i, ""),
        );
      }
    } catch {
      // ignore
    }
  };

  const handleImportPluginBundle = async (e: React.FormEvent) => {
    e.preventDefault();
    const rawPath = pluginImportPath.trim();
    const rawText = pluginImportText.trim();
    const fallbackName = pluginImportName.trim();
    if (!rawPath && !rawText && !fallbackName) {
      setPluginsBanner({
        type: "error",
        text: t("shell.pluginsPage.enterPluginPathErr"),
      });
      return;
    }

    try {
      if (rawPath && !rawText) {
        const res = await installPluginPackage({
          source_path: rawPath,
          fallback_name: fallbackName || undefined,
        });
        const installed = res?.installed_plugin;
        if (!installed) {
          throw new Error(res?.message || t("shell.pluginsPage.installFromPathFailed"));
        }
        const newPlugin: PluginSuiteItem = {
          id: installed.id,
          name: installed.name,
          version: installed.version,
          author: installed.author || "EOStage",
          license: installed.license || "Apache-2.0",
          description: installed.description,
          discipline: (installed.discipline as SkillDiscipline) || "custom",
          tags:
            installed.tags && installed.tags.length > 0
              ? installed.tags
              : [t("shell.pluginsPage.importLocalTag")],
          installPath: installed.install_path,
          sha256: installed.sha256,
          fileCount: installed.files?.length || 0,
          bundledSkills: (installed.bundled_skills || []).map((sk) => ({
            id: sk.id,
            name: sk.name,
            description: sk.description,
            prompt: sk.prompt,
          })),
          bundledConnector: installed.bundled_connector
            ? {
                id: installed.bundled_connector.id,
                name: installed.bundled_connector.name,
                transport:
                  installed.bundled_connector.transport === "stdio"
                    ? "stdio"
                    : "http",
                endpoint: installed.bundled_connector.endpoint,
                description: installed.bundled_connector.description,
              }
            : undefined,
          isOfficial: false,
          isCustomImport: true,
        };
        setBackendPluginMeta((prev) => ({
          ...prev,
          [installed.id]: installed,
        }));
        const nextCustom = [
          ...customPlugins.filter((c) => c.id !== newPlugin.id),
          newPlugin,
        ];
        setCustomPlugins(nextCustom);
        saveCustomPlugins(nextCustom);
        setInstalledPluginIds((prev) => {
          const next = new Set(prev);
          next.add(newPlugin.id);
          saveInstalledPluginIds(next);
          return next;
        });
        await syncPluginsFromBackend();
        setPluginImportModalOpen(false);
        setPluginImportName("");
        setPluginImportPath("");
        setPluginImportText("");
        setPluginFilter("added");
        setPluginsBanner({
          type: "success",
          text: t("shell.pluginsPage.installLocalSuccess", {
            name: installed.name,
            path: installed.install_path,
            sha: installed.sha256.slice(0, 12),
          }),
        });
        return;
      }

      const isDesktop = isTauriDesktopHost();
      let newPlugin: PluginSuiteItem;
      const importedSkillIds: string[] = [];

      if (rawText.startsWith("{")) {
        const parsed = JSON.parse(rawText);
        if (!isDesktop) {
          if (Array.isArray(parsed.skills) && parsed.skills.length > 0) {
            importCustomSkillsFromJson(JSON.stringify(parsed.skills));
            refreshSkills();
          }
          if (parsed.mcpServers && typeof parsed.mcpServers === "object") {
            const currentConns = loadCustomConnectors();
            const addedConns: CustomConnectorItem[] = [];
            for (const [serverKey, serverCfg] of Object.entries<any>(
              parsed.mcpServers,
            )) {
              addedConns.push({
                id: `plugin-mcp-${Date.now()}-${serverKey}`,
                name: serverCfg?.name || serverKey,
                transport: serverCfg?.command ? "stdio" : "http",
                endpoint:
                  serverCfg?.url ||
                  serverCfg?.endpoint ||
                  serverCfg?.command ||
                  "stdio://mcp",
                description:
                  serverCfg?.description ||
                  t("shell.pluginsPage.pluginMcpDesc", { key: serverKey }),
                enabled: true,
              });
            }
            if (addedConns.length > 0) {
              const merged = [...currentConns, ...addedConns];
              setCustomConnectors(merged);
              saveCustomConnectors(merged);
            }
          }
        }
        const pluginAuthor =
          typeof parsed.author === "string" && parsed.author.trim() ? parsed.author.trim() : "EOStage";
        newPlugin = {
          id: parsed.id || `custom-plugin-${Date.now()}`,
          name: parsed.name || fallbackName || t("shell.pluginsPage.customPluginFallbackName"),
          version: parsed.version || "1.0.0",
          author: pluginAuthor,
          description:
            parsed.description ||
            t("shell.pluginsPage.pluginPackJsonDesc"),
          discipline: "custom",
          skillIds: Array.isArray(parsed.skillIds)
            ? parsed.skillIds
            : importedSkillIds,
          connectorIds: ["builtin-mcp-workspace"],
          tags: [t("shell.pluginsPage.importLocalTag"), "Custom Plugin"],
          isOfficial: false,
          isCustomImport: true,
        };
      } else {
        if (!isDesktop && rawText) {
          importCustomSkillsFromJson(rawText);
          refreshSkills();
        }
        newPlugin = {
          id: `custom-plugin-${Date.now()}`,
          name: fallbackName || t("shell.pluginsPage.localPluginFallbackName"),
          version: "1.0.0",
          author: "EOStage",
          description: t("shell.pluginsPage.pluginSkillMdDesc"),
          discipline: "custom",
          skillIds: [],
          connectorIds: ["builtin-mcp-workspace"],
          tags: [t("shell.pluginsPage.importLocalTag"), "SKILL.md"],
          isOfficial: false,
          isCustomImport: true,
        };
      }

      if (!isDesktop) {
        const nextCustom = [
          ...customPlugins.filter((c) => c.id !== newPlugin.id),
          newPlugin,
        ];
        setCustomPlugins(nextCustom);
        saveCustomPlugins(nextCustom);

        setInstalledPluginIds((prev) => {
          const next = new Set(prev);
          next.add(newPlugin.id);
          saveInstalledPluginIds(next);
          return next;
        });
      }

      const lwId = rawPath
        ? startLongWork({
            title: t("shell.pluginsPage.importPluginTitle", { name: newPlugin.name }),
            category: "plugin",
            stageLabel: t("shell.pluginsPage.unpackingManifestStage"),
            onOpen: () => setActiveTab("plugins"),
          })
        : null;
      void installPluginPackage({
        source_path: rawPath || undefined,
        raw_content: rawText || JSON.stringify(newPlugin),
        fallback_name: newPlugin.name,
      })
        .then((res) => {
          const installed = res?.installed_plugin;
          if (installed) {
            setBackendPluginMeta((prev) => ({
              ...prev,
              [installed.id]: installed,
            }));
            void syncPluginsFromBackend();
          }
          if (isDesktop) {
            setPluginImportModalOpen(false);
            setPluginImportName("");
            setPluginImportPath("");
            setPluginImportText("");
            setPluginFilter("added");
            setPluginsBanner({
              type: "success",
              text: t("shell.pluginsPage.importAndEnabledToast", {
                name: installed?.name || newPlugin.name,
              }),
            });
          }
          if (lwId) {
            completeLongWork(lwId, {
              doneTitle: t("shell.pluginsPage.importPluginDone", { name: installed?.name || newPlugin.name }),
              doneSubtitle: t("shell.pluginsPage.enabledClickView"),
              onOpen: () => setActiveTab("plugins"),
            });
          }
        })
        .catch((err: any) => {
          const msg = err?.message || t("shell.pluginsPage.importPluginFailed", { name: newPlugin.name });
          if (lwId) failLongWork(lwId, msg);
          setPluginsBanner({ type: "error", text: msg });
        });

      if (!isDesktop) {
        setPluginImportModalOpen(false);
        setPluginImportName("");
        setPluginImportPath("");
        setPluginImportText("");
        setPluginFilter("added");
        setPluginsBanner({
          type: "success",
          text: t("shell.pluginsPage.importAndEnabledToast", { name: newPlugin.name }),
        });
      }
    } catch (err: any) {
      setPluginsBanner({
        type: "error",
        text:
          err?.context ||
          err?.message ||
          String(err || t("shell.pluginsPage.parseInstallPluginFailed")),
      });
    }
  };

  return (
    <div
      ref={rootRef}
      data-testid="plugins-and-skills-page"
      data-surface={isSettingsSurface ? "settings" : "workspace"}
      className="flex flex-1 min-w-0 h-full flex-col overflow-hidden bg-sp-surface select-none"
    >
      {/* 顶部标题与自解释能力页签：工作区聚焦「分析技能」与「扩展与工具连接」，运行组件与系统工具已归位至「设置 → 运行环境与工具」 */}
      <header className="shrink-0 border-b border-sp-line bg-sp-canvas px-6 sm:px-7 py-3.5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="min-w-0">
            <h1 className="text-[14px] font-semibold text-sp-text tracking-[-0.01em] leading-tight whitespace-nowrap">
              {isSettingsSurface
                ? t("shell.pluginsPage.settingsSectionTitle")
                : t("shell.nav.plugins")}
            </h1>
          </div>

          {!isSettingsSurface && (
            <div className="flex items-center gap-2.5 flex-wrap shrink-0">
              <div className="eo-segmented-track whitespace-nowrap">
                <button
                  type="button"
                  data-testid="tab-skills"
                  data-active={activeTab === "skills"}
                  onClick={() => setActiveTab("skills")}
                  title={t("shell.pluginsPage.analysisSkillsDesc")}
                  className="eo-segmented-pill gap-1.5 px-3 text-[12px] whitespace-nowrap cursor-pointer"
                >
                  <Puzzle className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
                  <span>{t("shell.pluginsPage.analysisSkillsTab")}</span>
                  <span className="text-xs text-sp-text-muted font-normal tabular-nums">
                    {skills.length}
                  </span>
                </button>

                <button
                  type="button"
                  data-testid="tab-extensions"
                  data-active={activeTab === "plugins" || activeTab === "tools"}
                  onClick={() => {
                    if (activeTab === "skills") {
                      setActiveTab("plugins");
                    }
                  }}
                  title={t("shell.pluginsPage.extensionsToolsDesc")}
                  className="eo-segmented-pill gap-1.5 px-3 text-[12px] whitespace-nowrap cursor-pointer"
                >
                  <Package className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
                  <span>{t("shell.pluginsPage.extensionsToolsTab")}</span>
                  <span className="text-xs text-sp-text-muted font-normal tabular-nums">
                    {totalAddedPluginCount + customConnectors.length}
                  </span>
                </button>
              </div>
            </div>
          )}
        </div>

        {!isSettingsSurface && (activeTab === "plugins" || activeTab === "tools") && (
          <div className="mt-3 pt-2.5 border-t border-sp-line/70 flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 flex-wrap">
              <button
                type="button"
                data-testid="tab-plugins"
                data-active={activeTab === "plugins"}
                onClick={() => setActiveTab("plugins")}
                className={cn(
                  "inline-flex h-7 items-center gap-1.5 rounded-sp-sm border px-2.5 text-xs whitespace-nowrap transition-colors cursor-pointer",
                  activeTab === "plugins"
                    ? "border-sp-text-secondary bg-sp-card font-semibold text-sp-text"
                    : "border-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
                )}
              >
                <Package className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
                <span>{t("shell.pluginsPage.pluginPackagesSubtab")}</span>
                <span className="text-xs text-sp-text-muted font-normal tabular-nums">
                  {totalAddedPluginCount}
                </span>
              </button>

              <button
                type="button"
                data-testid="tab-tools"
                data-active={activeTab === "tools"}
                onClick={() => setActiveTab("tools")}
                className={cn(
                  "inline-flex h-7 items-center gap-1.5 rounded-sp-sm border px-2.5 text-xs whitespace-nowrap transition-colors cursor-pointer",
                  activeTab === "tools"
                    ? "border-sp-text-secondary bg-sp-card font-semibold text-sp-text"
                    : "border-sp-line bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
                )}
              >
                <Cable className="h-3.5 w-3.5 shrink-0" strokeWidth={1.75} />
                <span>{t("shell.pluginsPage.mcpExternalSubtab")}</span>
                <span className="text-xs text-sp-text-muted font-normal tabular-nums">
                  {customConnectors.length}
                </span>
              </button>
            </div>

            {onOpenRuntimeSettings && (
              <button
                type="button"
                data-testid="open-runtime-tools-settings-btn"
                onClick={onOpenRuntimeSettings}
                className="text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:underline cursor-pointer"
              >
                {t("shell.pluginsPage.runtimeEnvLink")}
              </button>
            )}
          </div>
        )}
      </header>

      {/* 主工作区 */}
      <main
        ref={mainRef}
        className="flex-1 overflow-y-auto px-7 py-5 min-h-0 bg-sp-surface"
      >
        {/* ==================================================== */}
        {/* 页签 ① 技能 (Skills) */}
        {/* ==================================================== */}
        {activeTab === "skills" && (
          <div
            data-testid="skills-panel"
            className="max-w-7xl flex flex-col lg:grid lg:grid-cols-[216px_minmax(0,1fr)] gap-5 items-start"
          >
            {/* 左侧能力分类侧栏（含实时计数） */}
            <aside
              data-testid="skills-category-sidebar"
              aria-label={t("shell.pluginsPage.categoryAria")}
              className="w-full lg:sticky lg:top-4 rounded-sp-md border border-sp-line bg-sp-card p-2.5 space-y-1.5"
            >
              <div className="text-xs font-semibold text-sp-text-muted px-2 pt-0.5 pb-1 select-none">
                {t("shell.pluginsPage.categoryHeading")}
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-3 lg:flex lg:flex-col gap-1">
                {getDisciplineFilters(t).map((f) => {
                  const isActive = skillFilter === f.id;
                  const count = skillFilterCounts[f.id] ?? 0;
                  return (
                    <button
                      key={f.id}
                      type="button"
                      data-testid={`skill-filter-${f.id}`}
                      data-active={isActive}
                      onClick={() => setSkillFilter(f.id)}
                      className={cn(
                        "w-full flex items-center justify-between gap-2 rounded-sp-sm px-2.5 py-2 text-xs transition-colors cursor-pointer border",
                        isActive
                          ? "bg-sp-surface text-sp-text font-semibold border-sp-line shadow-xs"
                          : "bg-transparent text-sp-text-secondary border-transparent hover:bg-sp-hover hover:text-sp-text",
                      )}
                    >
                      <span className="truncate text-left">{f.label}</span>
                      <span className="text-xs text-sp-text-muted font-normal shrink-0 tabular-nums">
                        {count}
                      </span>
                    </button>
                  );
                })}
              </div>
            </aside>

            {/* 右侧主内容区：搜索工具栏 + 3 列技能卡片 */}
            <div className="min-w-0 w-full space-y-5">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-center gap-2.5 flex-1 min-w-[220px]">
                  <div className="relative flex-1 max-w-sm">
                    <Search
                      className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-sp-text-muted"
                      strokeWidth={1.75}
                    />
                    <input
                      type="text"
                      value={skillSearch}
                      onChange={(e) => setSkillSearch(e.target.value)}
                      placeholder={t("shell.pluginsPage.searchGeosciencePlaceholder")}
                      data-testid="skills-search-input"
                      className="h-8 w-full rounded-[7px] border border-sp-line bg-sp-canvas pl-8 pr-3 text-xs text-sp-text placeholder:text-sp-text-muted focus:border-sp-text-secondary focus:bg-sp-card focus:outline-none"
                    />
                  </div>
                  <span className="text-xs text-sp-text-muted shrink-0 hidden sm:inline tabular-nums">
                    {t("shell.pluginsPage.itemCountTotal", { count: filteredSkills.length })}
                  </span>
                </div>

                <div className="flex items-center gap-2 shrink-0 flex-wrap">
                  <button
                    type="button"
                    onClick={handleOpenCreateSkill}
                    data-testid="create-skill-btn"
                    className="inline-flex h-8 items-center gap-1 rounded-[7px] bg-[#18181b] px-3 text-xs font-medium text-white hover:bg-[#27272a] transition-colors cursor-pointer dark:bg-[#f4f4f5] dark:text-[#18181b]"
                  >
                    <Plus className="h-3.5 w-3.5" strokeWidth={1.75} />
                    <span>{t("shell.pluginsPage.newSkillBtn")}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setAddSkillByUrlOpen((v) => !v);
                      setImportPanelOpen(false);
                    }}
                    data-testid="add-skill-by-url-btn"
                    className="inline-flex h-8 items-center gap-1 rounded-[7px] border border-sp-line bg-sp-card px-3 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                  >
                    <Plus className="h-3.5 w-3.5" strokeWidth={1.75} />
                    <span>{t("shell.pluginsPage.addByUrlBtn")}</span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setImportPanelOpen((v) => !v);
                      setAddSkillByUrlOpen(false);
                    }}
                    data-testid="import-skills-btn"
                    className="inline-flex h-8 items-center rounded-[7px] border border-sp-line bg-sp-card px-3 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                  >
                    {t("shell.pluginsPage.importLocalBtn")}
                  </button>
                  <button
                    type="button"
                    onClick={() => void handleImportSkillFolder()}
                    disabled={importingSkillFolder}
                    data-testid="btn-import-skill-folder"
                    className="inline-flex h-8 items-center gap-1 rounded-[7px] border border-sp-line bg-sp-card px-3 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer disabled:opacity-50"
                  >
                    <FolderOpen className="h-3.5 w-3.5" strokeWidth={1.75} />
                    <span>
                      {importingSkillFolder
                        ? t("shell.pluginsPage.importingBtn")
                        : t("shell.pluginsPage.importFolderBtn")}
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={handleExportSkills}
                    data-testid="export-skills-btn"
                    className="inline-flex h-8 items-center rounded-[7px] border border-sp-line bg-sp-card px-3 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                  >
                    {t("shell.pluginsPage.exportCustomBtn")}
                  </button>
                </div>
              </div>

            {skillsBanner && (
              <div
                data-testid="skills-feedback-banner"
                className={cn(
                  "flex items-center justify-between gap-2 rounded-sp-sm border px-3 py-2 text-xs",
                  skillsBanner.type === "success"
                    ? "bg-sp-success-bg text-sp-success-text border-sp-line"
                    : "bg-sp-danger-bg text-sp-danger-text border-sp-line",
                )}
              >
                <span>{skillsBanner.text}</span>
                <button
                  type="button"
                  onClick={() => setSkillsBanner(null)}
                  className="text-current opacity-70 hover:opacity-100 cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                </button>
              </div>
            )}

            {/* 通过链接添加技能面板 */}
            {addSkillByUrlOpen && (
              <form
                onSubmit={handleAddSkillByUrl}
                data-testid="add-skill-by-url-panel"
                className="eo-settings-card"
              >
                <div className="eo-settings-card-content p-4 space-y-3">
                  <div className="flex items-center justify-between gap-2">
                    <div>
                      <span className="text-xs font-semibold text-sp-text">
                        {t("shell.pluginsPage.addByUrlPanelTitle")}
                      </span>
                      <p className="text-xs text-sp-text-muted mt-0.5">
                        {t("shell.pluginsPage.addByUrlPanelDesc")}
                      </p>
                    </div>
                    <button
                      type="button"
                      onClick={() => setAddSkillByUrlOpen(false)}
                      className="text-sp-text-muted hover:text-sp-text cursor-pointer"
                    >
                      <X className="h-4 w-4" strokeWidth={1.75} />
                    </button>
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      value={skillUrlInput}
                      onChange={(e) => setSkillUrlInput(e.target.value)}
                      placeholder="https://raw.githubusercontent.com/eostage/skills/main/sar-flood-mapping/SKILL.md"
                      data-testid="add-skill-by-url-input"
                      className="eo-settings-input flex-1 font-mono"
                    />
                    <button
                      type="submit"
                      disabled={addingSkillByUrl}
                      data-testid="confirm-add-skill-by-url-btn"
                      className="eo-settings-btn-primary h-8 px-3 disabled:opacity-50"
                    >
                      {addingSkillByUrl
                        ? t("shell.pluginsPage.addByUrlAdding")
                        : t("shell.pluginsPage.addByUrlConfirm")}
                    </button>
                  </div>
                </div>
                <div className="eo-settings-card-beard">
                  <span>{t("shell.pluginsPage.addByUrlHint")}</span>
                </div>
              </form>
            )}

            {/* 导入本地 SKILL.md 或 JSON 面板 */}
            {importPanelOpen && (
              <form
                onSubmit={handleConfirmImportSkills}
                data-testid="import-skills-panel"
                className="eo-settings-card"
              >
                <div className="eo-settings-card-content p-4 space-y-3">
                  <div className="flex items-center justify-between gap-2 flex-wrap">
                    <div>
                      <span className="text-xs font-semibold text-sp-text">
                        {t("shell.pluginsPage.importLocalPanelTitle")}
                      </span>
                      <p className="text-xs text-sp-text-muted mt-0.5">
                        {t("shell.pluginsPage.importLocalPanelDesc")}
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      <label className="eo-settings-btn-secondary cursor-pointer">
                        <FolderOpen className="h-3.5 w-3.5 text-sp-text-secondary" strokeWidth={1.75} />
                        <span>{t("shell.pluginsPage.chooseLocalFile")}</span>
                        <input
                          type="file"
                          accept=".md,.json,.txt,.yaml,.yml"
                          onChange={handleLocalSkillFileUpload}
                          className="hidden"
                        />
                      </label>
                      <button
                        type="button"
                        onClick={() => setImportPanelOpen(false)}
                        className="text-sp-text-muted hover:text-sp-text cursor-pointer"
                      >
                        <X className="h-4 w-4" strokeWidth={1.75} />
                      </button>
                    </div>
                  </div>
                  <textarea
                    rows={5}
                    value={importJsonText}
                    onChange={(e) => setImportJsonText(e.target.value)}
                    placeholder={t("shell.pluginsPage.importLocalTextareaPlaceholder")}
                    data-testid="import-skills-textarea"
                    className="w-full rounded-sp-sm border border-sp-line bg-sp-canvas p-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:bg-sp-card focus:outline-none"
                  />
                </div>
                <div className="eo-settings-card-beard">
                  <span>{t("shell.pluginsPage.importLocalSavedHint")}</span>
                  <div className="flex justify-end gap-2">
                    <button
                      type="button"
                      onClick={() => setImportPanelOpen(false)}
                      className="eo-settings-btn-secondary px-3"
                    >
                      {t("shell.pluginsPage.cancel")}
                    </button>
                    <button
                      type="submit"
                      data-testid="confirm-import-skills-btn"
                      className="eo-settings-btn-primary px-3"
                    >
                      {t("shell.pluginsPage.save")}
                    </button>
                  </div>
                </div>
              </form>
            )}

            {/* ============================================================== */}
            {/* 多文件地球科学技能包查看器 Modal 弹窗 (Skill Package Inspector Modal) */}
            {/* ============================================================== */}
            {inspectedSkill && (
              <ModalOverlay
                closeOnBackdropClick={false}
                onClose={() => setInspectedSkillId(null)}
                role="dialog"
                aria-modal={true}
                aria-labelledby="skill-inspector-modal-title"
                data-testid="skill-inspector-overlay"
                className="p-4 sm:p-6"
              >
                <div
                  data-testid="skill-package-inspector"
                  className="relative flex flex-col w-[min(920px,92vw)] h-[min(640px,84vh)] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-hidden animate-in zoom-in-95 duration-150"
                >
                  <div className="flex items-start justify-between gap-4 border-b border-sp-line bg-sp-card px-5 py-3.5 shrink-0">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2 flex-wrap">
                        <Puzzle
                          className="h-4 w-4 text-sp-text-secondary shrink-0"
                          strokeWidth={1.75}
                        />
                        <h2
                          id="skill-inspector-modal-title"
                          className="text-base font-semibold text-sp-text truncate"
                        >
                          {inspectedSkill.name}
                        </h2>
                        <span className="rounded bg-sp-surface px-2 py-0.5 text-xs text-sp-text-secondary border border-sp-line whitespace-nowrap shrink-0">
                          {inspectedSkill.badge ||
                            inspectedSkill.sourceLabel ||
                            t("shell.pluginsPage.badgeOfficial")}
                        </span>
                      </div>
                      <p className="mt-1 text-xs text-sp-text-secondary line-clamp-2">
                        {inspectedSkill.description}
                      </p>
                    </div>

                    <div className="flex items-center gap-2 shrink-0">
                      <button
                        type="button"
                        data-testid="inspector-copy-skill-btn"
                        onClick={() => handleCopyPrompt(inspectedSkill)}
                        className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap shrink-0 cursor-pointer"
                      >
                        {copiedSkillId === inspectedSkill.id ? (
                          <span className="text-sp-success-text font-medium">
                            {t("shell.pluginsPage.copied")}
                          </span>
                        ) : (
                          <span>{t("shell.pluginsPage.copySkillMdBtn")}</span>
                        )}
                      </button>
                      <button
                        type="button"
                        data-testid="close-skill-inspector-btn"
                        aria-label={t("shell.pluginsPage.closeSkillDetailsAria")}
                        onClick={() => setInspectedSkillId(null)}
                        className="inline-flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-muted hover:text-sp-text hover:bg-sp-hover shrink-0 cursor-pointer"
                      >
                        <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                      </button>
                    </div>
                  </div>

                  {/* 左右双栏：外框锁定 w-[min(920px,92vw)] h-[min(640px,84vh)]，切换左侧文件仅内部滚动、零抖动 */}
                  <div className="grid grid-cols-1 md:grid-cols-[210px_minmax(0,1fr)] flex-1 min-h-0 overflow-hidden">
                    <div className="min-h-0 max-h-40 md:max-h-none md:h-full border-b md:border-b-0 md:border-r border-sp-line bg-sp-card/60 p-3 space-y-1 overflow-y-auto">
                    <button
                      type="button"
                      data-testid="inspector-node-overview"
                      onClick={() => setInspectorActiveNode("__overview__")}
                      className={cn(
                        "flex w-full items-center gap-2 rounded-sp-sm px-2.5 py-1.5 text-left text-xs transition-colors cursor-pointer",
                        inspectorActiveNode === "__overview__"
                          ? "bg-sp-surface text-sp-text font-semibold border border-sp-line/60"
                          : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
                      )}
                    >
                      <Info
                        className="h-3.5 w-3.5 text-sp-text-secondary shrink-0"
                        strokeWidth={1.75}
                      />
                      <span>{t("shell.pluginsPage.overviewLabel")}</span>
                    </button>

                    {inspectorFileTree.dirs.map(([dirName, dirFiles]) => (
                      <div key={dirName} className="pt-1">
                        <div className="flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium text-sp-text-muted">
                          <Folder
                            className="h-3.5 w-3.5 text-sp-text-muted shrink-0"
                            strokeWidth={1.75}
                          />
                          <span>{dirName}</span>
                        </div>
                        <div className="pl-3 space-y-0.5">
                          {dirFiles.map((df) => (
                            <button
                              key={df.path}
                              type="button"
                              data-testid={`inspector-file-${df.path}`}
                              onClick={() =>
                                handleSelectInspectorFile(
                                  inspectedSkill,
                                  df.path,
                                )
                              }
                              className={cn(
                                "flex w-full items-center gap-1.5 rounded-sp-sm px-2 py-1 text-left font-mono text-xs transition-colors cursor-pointer truncate",
                                inspectorActiveNode === df.path
                                  ? "bg-sp-surface text-sp-text font-semibold border border-sp-line/60"
                                  : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
                              )}
                              title={df.path}
                            >
                              <FileText
                                className="h-3 w-3 text-sp-text-muted shrink-0"
                                strokeWidth={1.75}
                              />
                              <span className="truncate">
                                {df.path.slice(dirName.length + 1)}
                              </span>
                            </button>
                          ))}
                        </div>
                      </div>
                    ))}

                    <div className="pt-1 space-y-0.5">
                      {inspectorFileTree.rootFiles.map((rf) => (
                        <button
                          key={rf.path}
                          type="button"
                          data-testid={`inspector-file-${rf.path}`}
                          onClick={() =>
                            handleSelectInspectorFile(inspectedSkill, rf.path)
                          }
                          className={cn(
                            "flex w-full items-center gap-2 rounded-sp-sm px-2.5 py-1.5 text-left font-mono text-xs transition-colors cursor-pointer",
                            inspectorActiveNode === rf.path
                              ? "bg-sp-surface text-sp-text font-semibold border border-sp-line/60"
                              : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover",
                          )}
                        >
                          <FileCode
                            className="h-3.5 w-3.5 text-sp-text-muted shrink-0"
                            strokeWidth={1.75}
                          />
                          <span className="truncate">{rf.path}</span>
                        </button>
                      ))}
                    </div>
                  </div>

                  {/* 右侧内容面板：容器定高内部滚动，保证切换文件时外框零缩放 */}
                  <div
                    data-testid="inspector-content-pane"
                    className="min-h-0 min-w-0 h-full p-5 flex flex-col space-y-4 select-text overflow-y-auto"
                  >
                    {inspectorActiveNode === "__overview__" ? (
                      <div className="space-y-4">
                        <p className="text-xs sm:text-sm text-sp-text leading-relaxed">
                          {inspectedSkill.overview ||
                            inspectedSkill.description}
                        </p>

                        {/* 2×2 元数据网格：作者/来源 · 协议 · 版本 · 手动唤起 */}
                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-8 gap-y-2.5 border-y border-sp-line py-3 text-xs">
                          <div className="flex items-center justify-between border-b sm:border-b-0 border-sp-line/50 pb-1.5 sm:pb-0">
                            <span className="text-sp-text-muted">{t("shell.pluginsPage.authorSourceLabel")}</span>
                            <span className="font-medium text-sp-text">
                              {localizeSkillAuthor(resolveSkillAuthorDisplay(inspectedSkill), locale)}
                            </span>
                          </div>
                          <div className="flex items-center justify-between border-b sm:border-b-0 border-sp-line/50 pb-1.5 sm:pb-0">
                            <span className="text-sp-text-muted">{t("shell.pluginsPage.licenseLabel")}</span>
                            <span className="font-medium text-sp-text font-mono">
                              {inspectedSkill.license || "Custom"}
                            </span>
                          </div>
                          <div className="flex items-center justify-between">
                            <span className="text-sp-text-muted">{t("shell.pluginsPage.versionLabel")}</span>
                            <span className="font-mono text-sp-text-secondary">
                              {inspectedSkill.version || "1.0.0"}
                            </span>
                          </div>
                          <div className="flex items-center justify-between">
                            <span className="text-sp-text-muted">{t("shell.pluginsPage.manualInvokeLabel")}</span>
                            <code className="font-mono text-sp-text-secondary">
                              {inspectedSkill.slashCommand ||
                                `/${inspectedSkill.id}`}
                            </code>
                          </div>
                        </div>

                        {isThirdPartyBuiltinSkill(inspectedSkill) && (
                          <div
                            data-testid="inspector-third-party-attribution"
                            className="rounded-sp-sm bg-sp-card px-3 py-2 border border-sp-line text-xs text-sp-text-secondary"
                          >
                            <strong className="text-sp-text font-medium">
                              {t("shell.pluginsPage.thirdPartyAttribution")}
                            </strong>
                            {t("shell.pluginsPage.originalAuthorProtocol", {
                              author: localizeSkillAuthor(resolveSkillAuthorDisplay(inspectedSkill), locale),
                              license: inspectedSkill.license || "MIT",
                            })}
                            {t("shell.pluginsPage.licenseFooterHint")}
                          </div>
                        )}

                        {inspectedSkill.sharedComponentNote && (
                          <div className="rounded-sp-sm bg-sp-card px-3 py-2 border border-sp-line text-xs text-sp-text-secondary flex items-center justify-between gap-2 flex-wrap">
                            <span>
                              <strong className="text-sp-text font-medium">
                                {t("shell.pluginsPage.sharedDepsLabel")}
                              </strong>
                              {inspectedSkill.sharedComponentNote}
                            </span>
                            <span className="font-mono text-xs text-sp-text-muted">
                              runtime/skills/{inspectedSkill.id}/
                            </span>
                          </div>
                        )}

                        {inspectedSkill.exampleTasks &&
                          inspectedSkill.exampleTasks.length > 0 && (
                            <div className="space-y-2 pt-1">
                              <div className="text-xs font-medium text-sp-text-secondary">
                                {t("shell.pluginsPage.exampleTasksLabel")}
                              </div>
                              <div className="flex flex-col items-start gap-2">
                                {inspectedSkill.exampleTasks.map(
                                  (task, idx) => (
                                    <button
                                      key={idx}
                                      type="button"
                                      data-testid={`inspector-example-task-${idx}`}
                                      onClick={() =>
                                        handleCopyExampleTask(
                                          inspectedSkill,
                                          task,
                                        )
                                      }
                                      className="inline-flex items-center gap-2 rounded-full border border-sp-line bg-sp-card px-3.5 py-1.5 text-xs text-sp-text hover:bg-sp-hover transition-colors cursor-pointer text-left"
                                    >
                                      <span>{task}</span>
                                    </button>
                                  ),
                                )}
                              </div>
                            </div>
                          )}

                        <div className="space-y-1.5 pt-1">
                          <div className="flex items-center justify-between gap-2">
                            <span className="text-xs font-medium text-sp-text-secondary">
                              {t("shell.pluginsPage.promptContentLabel")}
                            </span>
                            <button
                              type="button"
                              onClick={() => handleCopyPrompt(inspectedSkill)}
                              className="inline-flex h-6 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                            >
                              {copiedSkillId === inspectedSkill.id
                                ? t("shell.pluginsPage.copied")
                                : t("shell.pluginsPage.copyBtn")}
                            </button>
                          </div>
                          <div
                            data-testid={`prompt-content-${inspectedSkill.id}`}
                            className="max-h-56 overflow-y-auto rounded-sp-sm bg-sp-card border border-sp-line p-3 font-mono text-xs text-sp-text leading-relaxed whitespace-pre-wrap select-text"
                          >
                            {inspectedSkill.prompt}
                          </div>
                        </div>
                      </div>
                    ) : (
                      <div className="space-y-2">
                        <div className="flex items-center justify-between border-b border-sp-line pb-2">
                          <code className="font-mono text-xs text-sp-text">
                            skills/{inspectedSkill.id}/{inspectorActiveNode}
                          </code>
                          <button
                            type="button"
                            onClick={async () => {
                              try {
                                await navigator.clipboard.writeText(
                                  currentInspectedFileText,
                                );
                              } catch {
                                // ignore
                              }
                            }}
                            className="inline-flex items-center rounded border border-sp-line bg-sp-card px-2 py-0.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                          >
                            {t("shell.pluginsPage.copyFileContent")}
                          </button>
                        </div>
                        {loadingFilePath === inspectorActiveNode ? (
                          <div className="py-8 text-center text-xs text-sp-text-muted">
                            {t("shell.pluginsPage.readingLocalSkillFile", { node: inspectorActiveNode })}
                          </div>
                        ) : (
                          <pre className="max-h-96 overflow-y-auto rounded-sp-sm bg-sp-card border border-sp-line p-3 font-mono text-xs text-sp-text leading-relaxed whitespace-pre-wrap">
                            {currentInspectedFileText}
                          </pre>
                        )}
                      </div>
                    )}
                  </div>
                </div>

                  {/* 底部状态与操作栏 */}
                  <div
                    data-testid="skill-inspector-footer"
                    className="flex items-center justify-between gap-3 border-t border-sp-line bg-sp-card px-5 py-3 shrink-0"
                  >
                    <span className="text-xs text-sp-text-muted truncate">
                      {t("shell.pluginsPage.authorFileCountFooter", {
                        author: localizeSkillAuthor(resolveSkillAuthorDisplay(inspectedSkill), locale),
                        count: inspectedSkill.files?.length || 1,
                      })}
                      {inspectedSkill.license ? ` · ${inspectedSkill.license}` : ""}
                    </span>
                    <div className="flex items-center gap-3 shrink-0">
                      <div className="inline-flex items-center gap-2 text-xs text-sp-text-secondary select-none">
                        <span>
                          {disabledSkillIds.has(inspectedSkill.id)
                            ? t("shell.pluginsPage.statusDisabled")
                            : t("shell.pluginsPage.statusEnabled")}
                        </span>
                        <button
                          type="button"
                          role="switch"
                          aria-checked={!disabledSkillIds.has(inspectedSkill.id)}
                          aria-label={
                            disabledSkillIds.has(inspectedSkill.id)
                              ? t("shell.pluginsPage.toggleEnableAria", { name: inspectedSkill.name })
                              : t("shell.pluginsPage.toggleDisableAria", { name: inspectedSkill.name })
                          }
                          data-state={!disabledSkillIds.has(inspectedSkill.id) ? "checked" : "unchecked"}
                          data-testid="inspector-toggle-enable-btn"
                          onClick={() => handleToggleSkillEnabled(inspectedSkill.id)}
                          className="eo-settings-switch shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                        >
                          <span className="eo-settings-switch-thumb" />
                        </button>
                      </div>
                      <button
                        type="button"
                        data-testid="inspector-done-btn"
                        onClick={() => setInspectedSkillId(null)}
                        className="inline-flex h-8 items-center rounded-sp-sm bg-[#18181b] px-3.5 text-xs font-medium text-white hover:bg-[#27272a] cursor-pointer dark:bg-[#f4f4f5] dark:text-[#18181b]"
                      >
                        {t("shell.pluginsPage.doneBtn")}
                      </button>
                    </div>
                  </div>
              </div>
            </ModalOverlay>
          )}

            {/* ============================================================== */}
            {/* 地球科学与空间分析技能卡片网格                                 */}
            {/* ============================================================== */}
            <div className="space-y-3">
              <h2 className="text-base font-semibold text-sp-text">
                {t("shell.pluginsPage.geoscienceSkillsHeading")}
              </h2>

              <div className="sp-grid-cards-md">
                {filteredSkills.map((rawSkill) => {
                  const skill = localizeAgentSkillForDisplay(rawSkill, locale);
                  const isCopied = copiedSkillId === skill.id;
                  const isEnabled = !disabledSkillIds.has(skill.id);
                  const fileCount = skill.files?.length || 1;
                  const displayAuthor = localizeSkillAuthor(resolveSkillAuthorDisplay(skill), locale);
                  const isThirdParty = isThirdPartyBuiltinSkill(skill);

                  return (
                    <div
                      key={skill.id}
                      data-testid={`skill-card-${skill.id}`}
                      className={cn(
                        "rounded-sp-md border bg-sp-surface p-4 transition-colors flex flex-col justify-between",
                        inspectedSkillId === skill.id
                          ? "border-sp-text-secondary bg-sp-card/40"
                          : "border-sp-line hover:bg-sp-card/30",
                      )}
                    >
                      <div>
                        {/* 卡片首行：左侧单色 Puzzle 图标 + 技能名（支持最多两行展示），右侧紧凑状态胶囊 */}
                        <div className="flex items-start justify-between gap-2">
                          <button
                            type="button"
                            onClick={() => handleInspectSkill(skill)}
                            className="inline-flex items-start gap-2 min-w-0 text-left cursor-pointer group"
                          >
                            <Puzzle
                              className="h-4 w-4 mt-0.5 text-sp-text-secondary shrink-0"
                              strokeWidth={1.75}
                            />
                            <span className="text-sm font-semibold text-sp-text leading-snug line-clamp-2 break-words group-hover:underline">
                              {skill.name}
                            </span>
                          </button>

                          <span
                            className={cn(
                              "shrink-0 whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium",
                              isEnabled
                                ? "border-sp-line bg-sp-success-bg text-sp-success-text"
                                : "border-sp-line bg-sp-card text-sp-text-muted",
                            )}
                          >
                            {isEnabled
                              ? t("shell.pluginsPage.statusEnabled")
                              : t("shell.pluginsPage.statusDisabled")}
                          </span>
                        </div>

                        <p className="mt-2 text-xs text-sp-text-secondary leading-relaxed line-clamp-2">
                          {skill.description}
                        </p>

                        <div className="mt-2.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-sp-text-muted">
                          <span className="break-words">
                            {isThirdParty
                              ? t("shell.pluginsPage.thirdPartyPrefix", { author: displayAuthor })
                              : displayAuthor}
                          </span>
                          <span aria-hidden="true">·</span>
                          <span className="whitespace-nowrap">
                            {t("shell.pluginsPage.fileCountUnit", { count: fileCount })}
                          </span>
                          {skill.license && (
                            <>
                              <span aria-hidden="true">·</span>
                              <span className="whitespace-nowrap">{skill.license}</span>
                            </>
                          )}
                        </div>
                      </div>

                      {/* 卡片底部操作栏：查看文件 + 复制 与 1-click 启停并排，保证窄屏不折行 */}
                      <div className="mt-3.5 pt-2.5 border-t border-sp-line/60">
                        <div className="flex items-center justify-between gap-1.5">
                          <div className="flex items-center gap-1.5 min-w-0">
                            <button
                              type="button"
                              onClick={() => handleInspectSkill(skill)}
                              data-testid={`inspect-skill-btn-${skill.id}`}
                              className="inline-flex h-7 shrink-0 whitespace-nowrap items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                            >
                              {t("shell.pluginsPage.viewFilesBtn")}
                            </button>

                            <button
                              type="button"
                              onClick={() => handleCopyPrompt(skill)}
                              title={t("shell.pluginsPage.copyPrompt")}
                              data-testid={`copy-skill-btn-${skill.id}`}
                              className="inline-flex h-7 shrink-0 whitespace-nowrap items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                            >
                              {isCopied ? (
                                <span className="text-sp-success-text font-medium">
                                  {t("shell.pluginsPage.copied")}
                                </span>
                              ) : (
                                <span>{t("shell.pluginsPage.copyBtn")}</span>
                              )}
                            </button>
                          </div>

                          <div className="flex items-center gap-2 shrink-0">
                            <button
                              type="button"
                              role="switch"
                              aria-checked={isEnabled}
                              aria-label={
                                isEnabled
                                  ? t("shell.pluginsPage.toggleDisableAria", { name: skill.name })
                                  : t("shell.pluginsPage.toggleEnableAria", { name: skill.name })
                              }
                              title={
                                isEnabled
                                  ? t("shell.pluginsPage.toggleDisableTitle")
                                  : t("shell.pluginsPage.toggleEnableTitle")
                              }
                              data-state={isEnabled ? "checked" : "unchecked"}
                              onClick={() => handleToggleSkillEnabled(skill.id)}
                              data-testid={`toggle-enable-skill-btn-${skill.id}`}
                              className="eo-settings-switch shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                            >
                              <span className="eo-settings-switch-thumb" />
                            </button>

                            {!skill.isBuiltin && (
                              <>
                                <button
                                  type="button"
                                  onClick={(e) => handleOpenEditSkill(skill, e)}
                                  title={t("shell.pluginsPage.edit")}
                                  data-testid={`edit-skill-btn-${skill.id}`}
                                  className="inline-flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                                >
                                  <Edit2 className="h-3 w-3" strokeWidth={1.75} />
                                </button>
                                <button
                                  type="button"
                                  onClick={(e) => handleDeleteSkill(skill.id, e.currentTarget)}
                                  title={t("shell.pluginsPage.delete")}
                                  data-testid={`delete-skill-btn-${skill.id}`}
                                  className="inline-flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-danger-text hover:bg-sp-danger-bg transition-colors cursor-pointer"
                                >
                                  <Trash2 className="h-3 w-3" strokeWidth={1.75} />
                                </button>
                              </>
                            )}
                          </div>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>

              {filteredSkills.length === 0 && (
                <EmptyState
                  kind="no-results"
                  size="page"
                  visual={<EmptyStateWireframe variant="no-results" />}
                  title={
                    skillSearch.trim()
                      ? t("shell.pluginsPage.noMatchingSkillsForQuery", { query: skillSearch.trim() })
                      : t("shell.pluginsPage.noMatchingSkills")
                  }
                  description={t("shell.pluginsPage.checkSpellingHint")}
                  primaryAction={
                    skillSearch.trim() || skillFilter !== "all"
                      ? {
                          label: t("shell.pluginsPage.clearFiltersBtn"),
                          onClick: () => {
                            setSkillSearch("");
                            setSkillFilter("all");
                          },
                        }
                      : undefined
                  }
                />
              )}
            </div>

            {/* 扩展库 (eostage-hub) 可用技能区域 — 对齐技能卡片网格，未安装优先展示 */}
            <div
              data-testid="hub-skills-section"
              className="pt-2 space-y-3"
            >
              <div className="flex items-center justify-between gap-2 flex-wrap">
                <div className="flex items-center gap-2 flex-wrap">
                  <h2 className="text-base font-semibold text-sp-text">
                    {t("shell.pluginsPage.hubSkillsHeading")}
                  </h2>
                  {hubSkillStats.total > 0 && (
                    <span className="text-xs text-sp-text-muted tabular-nums">
                      {t("shell.pluginsPage.hubStatsSummary", {
                        uninstalled: hubSkillStats.uninstalled,
                        installed: hubSkillStats.installed,
                      })}
                    </span>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => void handleLoadRegistry(undefined, true)}
                  disabled={registryLoading}
                  aria-busy={registryLoading}
                  data-testid="load-hub-skills-btn"
                  className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                >
                  <RefreshCw className={cn("h-3.5 w-3.5 shrink-0", registrySpinClass)} strokeWidth={1.75} />
                  <span>{t("shell.pluginsPage.refreshHubBtn")}</span>
                </button>
              </div>

              {hubSkillEntries.length > 0 ? (
                <div className="sp-grid-cards-md">
                  {hubSkillEntries.map((entry) => {
                    const alreadyInstalled = isHubSkillInstalled(entry, skills);
                    return (
                      <div
                        key={entry.id}
                        data-testid={`hub-skill-card-${entry.id}`}
                        className="rounded-sp-md border border-sp-line bg-sp-surface p-4 transition-colors hover:bg-sp-card/30 flex flex-col justify-between"
                      >
                        <div>
                          <div className="flex items-start justify-between gap-2">
                            <button
                              type="button"
                              onClick={() => setInspectedHubSkill(entry)}
                              className="inline-flex items-start gap-2 min-w-0 text-left cursor-pointer group"
                            >
                              <Puzzle
                                className="h-4 w-4 mt-0.5 text-sp-text-secondary shrink-0"
                                strokeWidth={1.75}
                              />
                              <span className="text-sm font-semibold text-sp-text leading-snug line-clamp-2 break-words group-hover:underline">
                                {entry.name}
                              </span>
                            </button>

                            <span
                              data-testid={`hub-skill-status-${entry.id}`}
                              className={cn(
                                "shrink-0 whitespace-nowrap rounded-full border px-2 py-0.5 text-xs font-medium",
                                alreadyInstalled
                                  ? "border-sp-line bg-sp-success-bg text-sp-success-text"
                                  : "border-sp-line bg-sp-card text-sp-text-muted",
                              )}
                            >
                              {alreadyInstalled
                                ? t("shell.pluginsPage.skillInstalled")
                                : t("shell.pluginsPage.notInstalledShort")}
                            </span>
                          </div>

                          <p className="mt-2 text-xs text-sp-text-secondary leading-relaxed line-clamp-2">
                            {entry.description}
                          </p>

                          <div className="mt-2.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-sp-text-muted">
                            <span>{t("shell.pluginsPage.officialHubAuthor")}</span>
                            <span aria-hidden="true">·</span>
                            <span className="font-mono whitespace-nowrap">v{entry.version}</span>
                            {entry.capabilities && entry.capabilities.length > 0 && (
                              <>
                                <span aria-hidden="true">·</span>
                                <span className="truncate">{entry.capabilities.slice(0, 2).join(" / ")}</span>
                              </>
                            )}
                          </div>
                        </div>

                        <div className="mt-3.5 pt-2.5 border-t border-sp-line/60 flex items-center justify-between gap-2">
                          <button
                            type="button"
                            onClick={() => setInspectedHubSkill(entry)}
                            data-testid={`inspect-hub-skill-btn-${entry.id}`}
                            className="inline-flex h-7 shrink-0 whitespace-nowrap items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                          >
                            {t("shell.pluginsPage.viewOverviewBtn")}
                          </button>

                          <button
                            type="button"
                            disabled={alreadyInstalled}
                            onClick={() => {
                              if (!alreadyInstalled) {
                                handleInstallSkillFromHub(entry);
                              }
                            }}
                            data-testid={`install-hub-skill-btn-${entry.id}`}
                            className={cn(
                              "inline-flex h-7 shrink-0 whitespace-nowrap items-center rounded-sp-sm border border-sp-line px-2.5 text-xs font-medium transition-colors",
                              alreadyInstalled
                                ? "bg-sp-card text-sp-text-muted cursor-default"
                                : "bg-sp-text text-sp-surface hover:opacity-90 cursor-pointer",
                            )}
                          >
                            {alreadyInstalled
                              ? t("shell.pluginsPage.skillInstalled")
                              : t("shell.pluginsPage.installSkillFromHub")}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="rounded-sp-md border border-sp-line bg-sp-card p-4 text-xs text-sp-text-muted">
                  {registryLoading
                    ? t("shell.pluginsPage.loadingHubSkills")
                    : skillSearch.trim()
                      ? t("shell.pluginsPage.noHubSkillsForQuery", { query: skillSearch.trim() })
                      : t("shell.pluginsPage.clickRefreshHubHint")}
                </div>
              )}
            </div>
            </div>

            {/* 新建/编辑技能右侧对照抽屉 (带高对比度蒙版且防误触关闭) */}
            {skillModalOpen && (
              <ModalOverlay
                closeOnBackdropClick={false}
                onClose={() => requestCloseSkillDrawer(skillCloseBtnRef.current)}
                data-testid="skill-edit-overlay"
                className="justify-end items-stretch p-0"
              >
              <aside
                role="dialog"
                aria-modal="true"
                aria-label={
                  editingSkill
                    ? t("shell.pluginsPage.edit")
                    : t("shell.pluginsPage.newSkill")
                }
                data-testid="skill-edit-modal"
                className="flex h-full w-[min(92vw,460px)] rounded-l-[16px] flex-col border-l border-sp-line bg-sp-surface shadow-sp-modal dark:bg-sp-card"
              >
                <div className="flex items-center justify-between px-5 py-3.5 border-b border-sp-line shrink-0">
                  <h3 className="text-sm font-semibold text-sp-text">
                    {editingSkill
                      ? t("shell.pluginsPage.edit")
                      : t("shell.pluginsPage.newSkill")}
                  </h3>
                  <button
                    ref={skillCloseBtnRef}
                    type="button"
                    aria-label={t("shell.pluginsPage.cancel")}
                    onClick={(e) => requestCloseSkillDrawer(e.currentTarget)}
                    className="rounded-sp-sm p-1 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                  >
                    <X className="h-4 w-4" strokeWidth={1.75} />
                  </button>
                </div>

                <form
                  onSubmit={handleSaveSkill}
                  className="flex flex-1 min-h-0 flex-col justify-between overflow-y-auto p-5"
                >
                  <div className="space-y-3.5">
                    {formError && (
                      <div className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text border border-sp-line">
                        {formError}
                      </div>
                    )}

                    <div>
                      <label className="block text-xs font-semibold text-sp-text mb-1">
                        {t("shell.pluginsPage.skillName")} *
                      </label>
                      <input
                        ref={skillNameInputRef}
                        type="text"
                        value={formName}
                        onChange={(e) => setFormName(e.target.value)}
                        placeholder={t("shell.pluginsPage.drawerExampleName")}
                        data-testid="skill-name-input"
                        className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-sp-text mb-1">
                        {t("shell.pluginsPage.skillDesc")} *
                      </label>
                      <input
                        type="text"
                        value={formDesc}
                        onChange={(e) => setFormDesc(e.target.value)}
                        placeholder={t("shell.pluginsPage.drawerExampleDesc")}
                        data-testid="skill-desc-input"
                        className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-semibold text-sp-text mb-1">
                        {t("shell.pluginsPage.skillPrompt")} *
                      </label>
                      <textarea
                        rows={11}
                        value={formPrompt}
                        onChange={(e) => setFormPrompt(e.target.value)}
                        placeholder={t("shell.pluginsPage.drawerExamplePrompt")}
                        data-testid="skill-prompt-textarea"
                        className="w-full rounded-sp-sm border border-sp-line bg-sp-surface p-2.5 font-mono text-xs text-sp-text leading-relaxed focus:border-sp-text-secondary focus:outline-none"
                      />
                    </div>
                  </div>

                  <div className="mt-4 flex items-center justify-end gap-2 pt-3 border-t border-sp-line shrink-0">
                    <button
                      type="button"
                      onClick={(e) => requestCloseSkillDrawer(e.currentTarget)}
                      className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer"
                    >
                      {t("shell.pluginsPage.cancel")}
                    </button>
                    <button
                      type="submit"
                      data-testid="save-skill-submit-btn"
                      className="h-8 px-4 rounded-sp-sm border border-sp-line bg-sp-text text-xs font-medium text-sp-surface hover:opacity-90 cursor-pointer"
                    >
                      {t("shell.pluginsPage.save")}
                    </button>
                  </div>
                </form>
              </aside>
              </ModalOverlay>
            )}
          </div>
        )}

        {/* ==================================================== */}
        {/* 页签 ② 连接器与外部工具 (Connectors)                   */}
        {/* ==================================================== */}
        {activeTab === "tools" && (
          <div data-testid="tools-panel" className="max-w-6xl space-y-6">
            <div className="space-y-3">
              <div className="flex items-center justify-between gap-3 flex-wrap">
                <div>
                  <div className="flex items-center gap-1.5">
                    <h2 className="text-base font-semibold text-sp-text">
                      {t("shell.pluginsPage.myConnectorsTitle")}
                    </h2>
                    <span
                      data-testid="connectors-total-count"
                      className="text-xs text-sp-text-muted"
                    >
                      {customConnectors.length}
                    </span>
                  </div>
                  <p className="mt-0.5 text-xs text-sp-text-secondary">
                    {t("shell.pluginsPage.myConnectorsDesc")}
                  </p>
                </div>
                <button
                  type="button"
                  data-testid="add-custom-connector-btn"
                  onClick={() => setConnectorFormOpen((v) => !v)}
                  className="inline-flex h-8 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                >
                  <Plus className="h-3.5 w-3.5" strokeWidth={1.75} />
                  <span>{t("shell.pluginsPage.customConnectorBtn")}</span>
                </button>
              </div>

              {connectorFormOpen && (
                <form
                  onSubmit={handleAddCustomConnector}
                  data-testid="custom-connector-form"
                  className="rounded-sp-md border border-sp-line bg-sp-card p-4 space-y-3"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-semibold text-sp-text">
                      {t("shell.pluginsPage.addConnectorTitle")}
                    </span>
                    <button
                      type="button"
                      onClick={() => setConnectorFormOpen(false)}
                      className="text-sp-text-muted hover:text-sp-text cursor-pointer"
                    >
                      <X className="h-4 w-4" strokeWidth={1.75} />
                    </button>
                  </div>
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-2.5">
                    <input
                      type="text"
                      data-testid="custom-connector-name-input"
                      value={connName}
                      onChange={(e) => setConnName(e.target.value)}
                      placeholder={t("shell.pluginsPage.connectorNamePlaceholder")}
                      className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                    />
                    <SpSelect
                      data-testid="custom-connector-transport-select"
                      aria-label={t("shell.pluginsPage.connectorTransportAria")}
                      value={connTransport}
                      onChange={(v) => setConnTransport(v as "http" | "stdio")}
                      size="sm"
                      options={[
                        { value: "http", label: "HTTP / Streamable MCP" },
                        { value: "stdio", label: t("shell.pluginsPage.transportStdioLabel") },
                      ]}
                    />
                    <input
                      type="text"
                      data-testid="custom-connector-endpoint-input"
                      value={connEndpoint}
                      onChange={(e) => setConnEndpoint(e.target.value)}
                      placeholder={t("shell.pluginsPage.endpointPlaceholder")}
                      className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                    />
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <input
                      type="text"
                      data-testid="custom-connector-desc-input"
                      value={connDesc}
                      onChange={(e) => setConnDesc(e.target.value)}
                      placeholder={t("shell.pluginsPage.connectorDescPlaceholder")}
                      className="h-8 flex-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                    />
                    <button
                      type="submit"
                      data-testid="save-custom-connector-btn"
                      className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-text text-xs font-medium text-sp-surface hover:opacity-90 cursor-pointer"
                    >
                      {t("shell.pluginsPage.saveConnectorBtn")}
                    </button>
                  </div>
                </form>
              )}

              <div
                data-testid="connectors-grid"
                className="sp-grid-cards-md"
              >
                {customConnectors.length === 0 && (
                  <EmptyState
                    kind="first-use"
                    size="card"
                    testId="connectors-empty-state"
                    visual={<EmptyStateIcon name="plug" size={22} />}
                    title={t("shell.pluginsPage.noCustomConnectorsTitle")}
                    className="col-span-full"
                  />
                )}

                {customConnectors.map((cc) => {
                  return (
                    <div
                      key={cc.id}
                      data-testid={`custom-connector-card-${cc.id}`}
                      className="rounded-sp-md border border-sp-line bg-sp-surface p-4 flex flex-col justify-between gap-3"
                    >
                      <div>
                        <div className="flex items-center justify-between gap-2">
                          <div className="flex items-center gap-2 min-w-0">
                            <Cable
                              className="h-4 w-4 text-sp-text-secondary shrink-0"
                              strokeWidth={1.75}
                            />
                            <h3 className="text-xs font-semibold text-sp-text truncate">
                              {cc.name}
                            </h3>
                          </div>
                          <span
                            className={cn(
                              "text-xs font-medium",
                              cc.enabled
                                ? "text-sp-text-secondary"
                                : "text-sp-text-muted",
                            )}
                          >
                            {cc.enabled
                              ? t("shell.pluginsPage.customBadgeLabel", {
                                  transport: cc.transport.toUpperCase(),
                                })
                              : t("shell.pluginsPage.disabledBadgeLabel", {
                                  transport: cc.transport.toUpperCase(),
                                })}
                          </span>
                        </div>
                        <code className="mt-1 block truncate font-mono text-xs text-sp-text-muted">
                          {cc.endpoint}
                        </code>
                        <p className="mt-1.5 text-xs text-sp-text-secondary">
                          {cc.description}
                        </p>
                        {connectorProbeById[cc.id] && (
                          <div
                            data-testid={`custom-connector-tools-panel-${cc.id}`}
                            className={cn(
                              "mt-2.5 rounded-sp-sm border p-2.5 text-xs space-y-1.5",
                              connectorProbeById[cc.id].ok
                                ? "border-sp-line bg-sp-card text-sp-text-secondary"
                                : "border-sp-line bg-sp-danger-bg text-sp-danger-text",
                            )}
                          >
                            {connectorProbeById[cc.id].ok ? (
                              <>
                                <div className="flex items-center justify-between gap-2 font-semibold text-sp-text">
                                  <span>
                                    {t("shell.pluginsPage.discoveredToolsCount", {
                                      count: connectorProbeById[cc.id].tools.length,
                                    })}
                                    {connectorProbeById[cc.id].server_name
                                      ? ` (${connectorProbeById[cc.id].server_name})`
                                      : ""}
                                  </span>
                                  <code className="font-mono text-xs text-sp-text-muted">
                                    {connectorProbeById[cc.id].transport}
                                  </code>
                                </div>
                                {connectorProbeById[cc.id].tools.length === 0 ? (
                                  <p className="text-sp-text-muted">
                                    {t("shell.pluginsPage.handshakeNoTools")}
                                  </p>
                                ) : (
                                  <div className="max-h-40 overflow-y-auto space-y-1">
                                    {connectorProbeById[cc.id].tools.map((tl) => (
                                      <div
                                        key={tl.qualified_name}
                                        className="rounded border border-sp-line/70 bg-sp-surface px-2 py-1"
                                      >
                                        <code className="font-mono text-xs font-semibold text-sp-text break-all">
                                          {tl.qualified_name}
                                        </code>
                                        {tl.description && (
                                          <p className="mt-0.5 text-xs text-sp-text-secondary">
                                            {tl.description}
                                          </p>
                                        )}
                                      </div>
                                    ))}
                                  </div>
                                )}
                              </>
                            ) : (
                              <div>
                                {t("shell.pluginsPage.probeFailedPrefix")}
                                {connectorProbeById[cc.id].error || t("shell.pluginsPage.cannotConnectMcp")}
                              </div>
                            )}
                          </div>
                        )}
                      </div>
                      <div className="flex items-center justify-between gap-2 pt-2 border-t border-sp-line/60">
                        <div className="flex items-center gap-1.5 flex-wrap">
                          <button
                            type="button"
                            data-testid={`view-connector-usage-btn-${cc.id}`}
                            onClick={() => setInspectedConnectorId(cc.id)}
                            className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                          >
                            {t("shell.pluginsPage.viewUsageBtn")}
                          </button>
                          <button
                            type="button"
                            data-testid={`probe-custom-connector-btn-${cc.id}`}
                            disabled={probingConnectorId === cc.id}
                            onClick={() => void handleProbeConnectorTools(cc.id)}
                            className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                          >
                            {probingConnectorId === cc.id && (
                              <Loader2 className="h-3 w-3 animate-spin" strokeWidth={1.75} />
                            )}
                            <span>
                              {probingConnectorId === cc.id
                                ? t("shell.pluginsPage.probingBtn")
                                : t("shell.pluginsPage.viewToolsBtn")}
                            </span>
                          </button>
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            role="switch"
                            aria-checked={cc.enabled}
                            aria-label={
                              cc.enabled
                                ? t("shell.pluginsPage.disableConnectorAria", { name: cc.name })
                                : t("shell.pluginsPage.enableConnectorAria", { name: cc.name })
                            }
                            title={
                              cc.enabled
                                ? t("shell.pluginsPage.toggleDisableTitle")
                                : t("shell.pluginsPage.toggleEnableTitle")
                            }
                            data-state={cc.enabled ? "checked" : "unchecked"}
                            data-testid={`toggle-custom-connector-btn-${cc.id}`}
                            onClick={() => handleToggleCustomConnectorEnabled(cc.id)}
                            className="eo-settings-switch shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                          >
                            <span className="eo-settings-switch-thumb" />
                          </button>
                          <button
                            type="button"
                            data-testid={`delete-custom-connector-btn-${cc.id}`}
                            onClick={(e) => handleDeleteCustomConnector(cc.id, e.currentTarget)}
                            className="text-xs text-sp-danger-text hover:underline cursor-pointer"
                          >
                            {t("shell.pluginsPage.removeBtn")}
                          </button>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        )}

        {/* ==================================================== */}
        {/* 页签 ③ 插件市场 (Extension Marketplace) — 在线市场源浏览与已安装能力管理 */}
        {/* ==================================================== */}
        {activeTab === "plugins" && (
          <div data-testid="plugins-panel" className="max-w-5xl space-y-5">
            <div className="flex items-start justify-between gap-4 flex-wrap">
              <div>
                <h2 className="text-base font-semibold text-sp-text">
                  {t("shell.pluginsPage.pluginsOfficialTitle")}
                </h2>
                <div className="mt-3 flex items-center gap-2 flex-wrap">
                  <button
                    type="button"
                    data-testid="plugins-subtab-added"
                    onClick={() => setPluginFilter("added")}
                    className={cn(
                      "rounded-sp-sm px-3 py-1 text-xs whitespace-nowrap cursor-pointer transition-colors",
                      pluginFilter === "added"
                        ? "bg-sp-card text-sp-text font-semibold border border-sp-line"
                        : "text-sp-text hover:bg-sp-hover",
                    )}
                  >
                    {t("shell.pluginsPage.addedTabCount", { count: totalAddedPluginCount })}
                  </button>
                  <button
                    type="button"
                    data-testid="plugins-subtab-browse"
                    onClick={() => setPluginFilter("browse")}
                    className={cn(
                      "rounded-sp-sm px-3 py-1 text-xs whitespace-nowrap cursor-pointer transition-colors",
                      pluginFilter === "browse"
                        ? "bg-sp-card text-sp-text font-semibold border border-sp-line"
                        : "text-sp-text hover:bg-sp-hover",
                    )}
                  >
                    {t("shell.pluginsPage.browseMarketplaceCount", { count: browseBasePlugins.length })}
                  </button>
                </div>
              </div>

              <div className="flex items-center gap-2 flex-wrap">
                <div className="relative w-52">
                  <Search
                    className="absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-sp-text-muted"
                    strokeWidth={1.75}
                  />
                  <input
                    type="text"
                    value={pluginSearch}
                    onChange={(e) => setPluginSearch(e.target.value)}
                    placeholder={t("shell.pluginsPage.searchMarketplacePlaceholder")}
                    data-testid="plugins-search-input"
                    className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface pl-8 pr-3 text-xs text-sp-text placeholder:text-sp-text-muted focus:border-sp-text-secondary focus:outline-none"
                  />
                </div>

                <button
                  type="button"
                  data-testid="import-plugin-bundle-btn"
                  onClick={() => setPluginImportModalOpen((v) => !v)}
                  className="inline-flex h-8 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap shrink-0 cursor-pointer"
                  title={t("shell.pluginsPage.importZipTitle")}
                >
                  <FolderOpen
                    className="h-3.5 w-3.5 text-sp-text-secondary shrink-0"
                    strokeWidth={1.75}
                  />
                  <span>{t("shell.pluginsPage.importZipBtn")}</span>
                </button>
              </div>
            </div>

            {/* 市场源状态栏：如实展示远程注册表探测状态（同步中 / 远端验证连接 / 本地离线回退），绝不在未连接或离线时虚标已连接 */}
            <div
              data-testid="plugin-marketplace-sources-bar"
              data-registry-state={registryConnectionState}
              className="rounded-sp-md border border-sp-line bg-sp-card/70 px-4 py-2.5 flex items-center justify-between gap-3 flex-wrap text-xs"
            >
              <div className="flex items-center gap-2 flex-wrap">
                {registryConnectionState === "syncing" ? (
                  <>
                    <span className="font-semibold text-sp-text">{t("shell.pluginsPage.marketStatusLabel")}</span>
                    <span className="inline-flex items-center gap-1.5 rounded bg-sp-surface px-2 py-0.5 font-medium text-sp-text border border-sp-line">
                      <span>{t("shell.pluginsPage.syncingEllipsis")}</span>
                    </span>
                    <span className="text-sp-text-muted">
                      {t("shell.pluginsPage.connectingOfficialHub")}
                    </span>
                  </>
                ) : registryConnectionState === "connected" ? (
                  <>
                    <span className="font-semibold text-sp-text">{t("shell.pluginsPage.connectedMarketLabel")}</span>
                    <span className="inline-flex items-center gap-1.5 rounded bg-sp-surface px-2 py-0.5 font-medium text-sp-text border border-sp-line">
                      <CheckCircle2
                        className="h-3 w-3 text-sp-success-text"
                        strokeWidth={1.75}
                      />
                      <span>
                        {/^https?:\/\//i.test(registryReport?.source_url || "")
                          ? t("shell.pluginsPage.officialHubName")
                          : t("shell.pluginsPage.localManifestName")}
                      </span>
                    </span>
                    <span className="text-sp-text-muted">
                      {/^https?:\/\//i.test(registryReport?.source_url || "")
                        ? t("shell.pluginsPage.dualMirrorHint")
                        : t("shell.pluginsPage.offlineIndexHint")}
                    </span>
                  </>
                ) : (
                  <>
                    <span className="font-semibold text-sp-text">{t("shell.pluginsPage.currentCatalogLabel")}</span>
                    <span className="inline-flex items-center gap-1.5 rounded bg-sp-surface px-2 py-0.5 font-medium text-sp-text border border-sp-line">
                      <Package
                        className="h-3 w-3 text-sp-text-secondary"
                        strokeWidth={1.75}
                      />
                      <span>{t("shell.pluginsPage.localBuiltinFallbackLabel")}</span>
                    </span>
                    <span className="text-sp-text-muted">
                      {registryError
                        ? t("shell.pluginsPage.remoteUnreachableFallbackHint")
                        : t("shell.pluginsPage.unconnectedFallbackHint")}
                    </span>
                  </>
                )}
              </div>
              <div className="flex items-center gap-3 shrink-0">
                {hasLiveRegistryHost && (
                  <button
                    type="button"
                    data-testid="sync-plugin-registry-btn"
                    disabled={registryLoading}
                    aria-busy={registryLoading}
                    onClick={() => void handleLoadRegistry(undefined, true)}
                    className="inline-flex items-center gap-1 text-xs font-medium text-sp-text hover:underline disabled:opacity-50 whitespace-nowrap cursor-pointer"
                  >
                    <RefreshCw
                      className={cn("h-3.5 w-3.5 text-sp-text-secondary shrink-0", registrySpinClass)}
                      strokeWidth={1.75}
                    />
                    <span>{t("shell.pluginsPage.syncRemoteBtn")}</span>
                  </button>
                )}
              </div>
            </div>

            {/* 来源筛选胶囊（仅展示当前目录中真实存在的来源，避免空筛选项） */}
            <div className="flex items-center justify-between gap-2 flex-wrap">
              <div className="flex items-center gap-1.5 flex-wrap">
                {([
                  "all",
                  ...Array.from(
                    new Set(
                      (pluginFilter === "added"
                        ? allAvailablePlugins.filter((p) => installedPluginIds.has(p.id))
                        : browseBasePlugins
                      )
                        .map((p) => p.author)
                        .filter((a) => typeof a === "string" && a.trim().length > 0),
                    ),
                  ),
                ] as const).map(
                  (authorOpt) => (
                    <button
                      key={authorOpt}
                      type="button"
                      data-testid={`plugin-author-filter-${authorOpt}`}
                      onClick={() => setPluginAuthorFilter(authorOpt)}
                      className={cn(
                        "rounded-sp-sm px-2.5 py-1 text-xs border transition-colors cursor-pointer",
                        pluginAuthorFilter === authorOpt
                          ? "bg-sp-card text-sp-text font-semibold border-sp-text-secondary"
                          : "bg-sp-surface text-sp-text border-sp-line hover:bg-sp-hover",
                      )}
                    >
                      {authorOpt === "all"
                        ? t("shell.pluginsPage.allSourcesFilter")
                        : localizeSkillAuthor(authorOpt, locale)}
                    </button>
                  ),
                )}
              </div>
              <span className="text-xs text-sp-text-muted">
                {pluginFilter === "added"
                  ? t("shell.pluginsPage.installedPluginCount", { count: addedPlugins.length })
                  : t("shell.pluginsPage.availablePluginCount", { count: browsePlugins.length })}
              </span>
            </div>

            {pluginsBanner && (
              <div
                data-testid="plugins-feedback-banner"
                className={cn(
                  "flex items-center justify-between gap-2 rounded-sp-sm border px-3 py-2 text-xs",
                  pluginsBanner.type === "success"
                    ? "bg-sp-success-bg text-sp-success-text border-sp-line"
                    : "bg-sp-danger-bg text-sp-danger-text border-sp-line",
                )}
              >
                <span>{pluginsBanner.text}</span>
                <button
                  type="button"
                  onClick={() => setPluginsBanner(null)}
                  className="text-current opacity-70 hover:opacity-100 cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                </button>
              </div>
            )}

            {/* 导入插件弹窗面板（支持本地 .zip / 文件夹路径 / plugin.json / SKILL.md / mcp.json，统一 16px 圆角与防误触遮罩） */}
            {pluginImportModalOpen && (
              <ModalOverlay
                closeOnBackdropClick={false}
                onClose={() => setPluginImportModalOpen(false)}
                role="dialog"
                aria-modal={true}
                aria-label={t("shell.pluginsPage.pluginImportDialogAria")}
                data-testid="import-plugin-bundle-overlay"
                className="p-4 sm:p-6"
              >
              <form
                onSubmit={handleImportPluginBundle}
                data-testid="import-plugin-bundle-modal"
                className="relative flex flex-col w-[min(640px,92vw)] max-h-[84vh] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-y-auto p-5 space-y-3"
              >
                <div className="flex items-center justify-between gap-2 flex-wrap">
                  <div>
                    <span className="text-xs font-semibold text-sp-text">
                      {t("shell.pluginsPage.pluginImportTitle")}
                    </span>
                    <p className="text-xs text-sp-text-muted mt-0.5">
                      {t("shell.pluginsPage.pluginImportDesc")}
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <label className="inline-flex h-7 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer">
                      <FolderOpen
                        className="h-3.5 w-3.5 text-sp-text-secondary shrink-0"
                        strokeWidth={1.75}
                      />
                      <span>{t("shell.pluginsPage.selectFileZipBtn")}</span>
                      <input
                        type="file"
                        accept=".zip,.json,.md,.yaml,.yml"
                        multiple
                        onChange={handleLocalPluginFileUpload}
                        className="hidden"
                      />
                    </label>
                    <label className="inline-flex h-7 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer">
                      <FolderOpen
                        className="h-3.5 w-3.5 text-sp-text-secondary shrink-0"
                        strokeWidth={1.75}
                      />
                      <span>{t("shell.pluginsPage.selectPluginFolderBtn")}</span>
                      <input
                        type="file"
                        {...({ webkitdirectory: "", directory: "" } as Record<string, string>)}
                        multiple
                        onChange={handleLocalPluginFileUpload}
                        className="hidden"
                      />
                    </label>
                    <button
                      type="button"
                      aria-label={t("shell.pluginsPage.closeImportModalAria")}
                      onClick={() => setPluginImportModalOpen(false)}
                      className="text-sp-text-muted hover:text-sp-text cursor-pointer"
                    >
                      <X className="h-4 w-4" strokeWidth={1.75} />
                    </button>
                  </div>
                </div>

                <input
                  type="text"
                  value={pluginImportPath}
                  onChange={(e) => setPluginImportPath(e.target.value)}
                  placeholder={t("shell.pluginsPage.localPathPlaceholder")}
                  data-testid="import-plugin-path-input"
                  className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />

                <p
                  data-testid="import-plugin-security-hint"
                  className="rounded-sp-sm border border-sp-line/80 bg-sp-canvas px-2.5 py-1.5 text-xs leading-relaxed text-sp-text-muted"
                >
                  {t("shell.pluginsPage.pluginImportSecurityHint")}
                </p>

                <input
                  type="text"
                  value={pluginImportName}
                  onChange={(e) => setPluginImportName(e.target.value)}
                  placeholder={t("shell.pluginsPage.pluginNamePlaceholder")}
                  data-testid="import-plugin-name-input"
                  className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />

                <textarea
                  rows={5}
                  value={pluginImportText}
                  onChange={(e) => setPluginImportText(e.target.value)}
                  placeholder={t("shell.pluginsPage.pluginJsonPlaceholder")}
                  data-testid="import-plugin-textarea"
                  className="w-full rounded-sp-sm border border-sp-line bg-sp-surface p-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />

                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => setPluginImportModalOpen(false)}
                    className="h-7 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:text-sp-text whitespace-nowrap cursor-pointer"
                  >
                    {t("shell.pluginsPage.cancel")}
                  </button>
                  <button
                    type="submit"
                    data-testid="confirm-import-plugin-btn"
                    className="h-7 px-3 rounded-sp-sm border border-sp-line bg-sp-text text-xs font-medium text-sp-surface hover:opacity-90 whitespace-nowrap cursor-pointer"
                  >
                    {t("shell.pluginsPage.confirmImportBtn")}
                  </button>
                </div>
              </form>
              </ModalOverlay>
            )}

            {/* 视图 A：已添加插件列表 (pluginFilter === "added") */}
            {pluginFilter === "added" && (
              <div
                data-testid="plugins-added-view"
                className="grid grid-cols-1 gap-3.5"
              >
                {addedPlugins.map((suite) => {
                  const suiteSkills = getSuiteSkills(suite);
                  const isEnabled = !disabledPluginIds.has(suite.id);

                  return (
                    <div
                      key={suite.id}
                      data-testid={`plugin-suite-card-${suite.id}`}
                      className="rounded-sp-md border border-sp-line bg-sp-surface p-4 space-y-3"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2 flex-wrap">
                            <Package
                              className="h-4 w-4 text-sp-text-secondary shrink-0"
                              strokeWidth={1.75}
                            />
                            <h3 className="text-sm font-semibold text-sp-text">
                              {suite.name}
                            </h3>
                            <span className="rounded bg-sp-card px-2 py-0.5 text-xs text-sp-text-secondary border border-sp-line">
                              {localizeSkillAuthor(suite.author, locale)}
                            </span>
                            <code className="rounded bg-sp-card px-1.5 py-0.5 font-mono text-xs text-sp-text-muted">
                              v{suite.version}
                            </code>
                            {suite.isRetired && (
                              <span
                                data-testid={`plugin-retired-badge-${suite.id}`}
                                className="rounded bg-sp-card px-2 py-0.5 text-xs font-medium text-sp-text-muted border border-sp-line"
                              >
                                {t("shell.pluginsPage.archivedPluginBadge")}
                              </span>
                            )}
                            <span
                              className={cn(
                                "text-xs font-medium",
                                isEnabled
                                  ? "text-sp-success-text"
                                  : "text-sp-text-muted",
                              )}
                            >
                              {isEnabled
                                ? t("shell.pluginsPage.statusEnabled")
                                : t("shell.pluginsPage.stateDisabled")}
                            </span>
                          </div>
                          <p className="mt-1.5 text-xs text-sp-text-secondary leading-relaxed">
                            {suite.description}
                          </p>
                          {suite.installPath && (
                            <p
                              data-testid={`plugin-install-path-${suite.id}`}
                              className="mt-1 font-mono text-xs text-sp-text-muted truncate"
                            >
                              {t("shell.pluginsPage.installPathPrefix", { path: suite.installPath })}
                              {suite.sha256
                                ? ` · SHA-256: ${suite.sha256.slice(0, 12)}…`
                                : ""}
                            </p>
                          )}
                        </div>

                        <div className="flex items-center gap-2 shrink-0 flex-wrap">
                          <button
                            type="button"
                            data-testid={`plugin-view-skills-btn-${suite.id}`}
                            onClick={() => setPreviewPluginSuite(suite)}
                            className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                          >
                            {t("shell.pluginsPage.viewSuiteSkillsCount", { count: suiteSkills.length })}
                          </button>
                          <button
                            type="button"
                            role="switch"
                            aria-checked={isEnabled}
                            aria-label={
                              isEnabled
                                ? t("shell.pluginsPage.disablePluginAria", { name: suite.name })
                                : t("shell.pluginsPage.enablePluginAria", { name: suite.name })
                            }
                            title={
                              isEnabled
                                ? t("shell.pluginsPage.toggleDisableTitle")
                                : t("shell.pluginsPage.toggleEnableTitle")
                            }
                            data-state={isEnabled ? "checked" : "unchecked"}
                            data-testid={`plugin-toggle-btn-${suite.id}`}
                            onClick={() => handleTogglePluginSuite(suite)}
                            className="eo-settings-switch shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                          >
                            <span className="eo-settings-switch-thumb" />
                          </button>
                          <button
                            type="button"
                            data-testid={`plugin-remove-btn-${suite.id}`}
                            onClick={(e) => handleRemovePlugin(suite, e.currentTarget)}
                            className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-danger-text hover:bg-sp-danger-bg cursor-pointer"
                          >
                            {t("shell.pluginsPage.moveOutBtn")}
                          </button>
                        </div>
                      </div>

                      <div className="flex items-center gap-1.5 flex-wrap pt-2 border-t border-sp-line/60">
                        <span className="text-xs text-sp-text-muted mr-1">
                          {t("shell.pluginsPage.includedSkillsCount", { count: suiteSkills.length })}
                        </span>
                        {suiteSkills.map((sk) => (
                          <button
                            key={sk.id}
                            type="button"
                            onClick={() => {
                              setActiveTab("skills");
                              handleInspectSkill(sk);
                            }}
                            className="rounded bg-sp-card px-2 py-0.5 text-xs font-medium text-sp-text hover:bg-sp-hover border border-sp-line cursor-pointer"
                          >
                            {sk.name}
                          </button>
                        ))}
                      </div>
                    </div>
                  );
                })}

                {addedPlugins.length === 0 && (
                  <EmptyState
                    kind="first-use"
                    size="page"
                    testId="plugins-added-empty"
                    visual={<EmptyStateWireframe variant="plugin-grid" showAccentAction />}
                    title={t("shell.pluginsPage.noAddedPluginsTitle")}
                    className="rounded-sp-md border border-sp-line bg-sp-surface"
                  />
                )}
              </div>
            )}

            {/* 视图 B：浏览插件目录视图 (pluginFilter === "browse") */}
            {pluginFilter === "browse" && (
              <div
                data-testid="plugins-browse-view"
                className="sp-grid-cards-lg"
              >
                {browsePlugins.map((suite) => {
                  const suiteSkills = getSuiteSkills(suite);
                  const isInstalled = installedPluginIds.has(suite.id);

                  return (
                    <div
                      key={suite.id}
                      data-testid={`browse-plugin-card-${suite.id}`}
                      className="rounded-sp-md border border-sp-line bg-sp-surface p-4 flex flex-col justify-between gap-3"
                    >
                      <div>
                        <div className="flex items-start justify-between gap-2">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2 flex-wrap">
                              <Package
                                className="h-4 w-4 text-sp-text-secondary shrink-0"
                                strokeWidth={1.75}
                              />
                              <h3 className="text-sm font-semibold text-sp-text">
                                {suite.name}
                              </h3>
                            </div>
                            <div className="mt-1 flex items-center gap-2 text-xs text-sp-text-muted flex-wrap">
                              <span>
                                {t("shell.pluginsPage.authorPrefix", {
                                  author: localizeSkillAuthor(suite.author, locale),
                                })}
                              </span>
                              <span>·</span>
                              <span className="font-mono">v{suite.version}</span>
                              <span>·</span>
                              <span>{t("shell.pluginsPage.skillsCountBadge", { count: suiteSkills.length })}</span>
                            </div>
                          </div>

                          <span
                            className={cn(
                              "shrink-0 rounded px-2 py-0.5 text-xs font-medium border",
                              isInstalled
                                ? "bg-sp-success-bg text-sp-success-text border-sp-line"
                                : "bg-sp-card text-sp-text-secondary border-sp-line",
                            )}
                          >
                            {isInstalled
                              ? t("shell.pluginsPage.addedBadge")
                              : t("shell.pluginsPage.addableBadge")}
                          </span>
                        </div>

                        <p className="mt-2 text-xs text-sp-text-secondary leading-relaxed">
                          {suite.description}
                        </p>

                        {suite.tags && suite.tags.length > 0 && (
                          <div className="mt-2.5 flex items-center gap-1.5 flex-wrap">
                            {suite.tags.map((tag) => (
                              <span
                                key={tag}
                                className="rounded bg-sp-card px-2 py-0.5 text-xs text-sp-text-muted border border-sp-line/70"
                              >
                                {tag}
                              </span>
                            ))}
                          </div>
                        )}
                      </div>

                      <div className="flex items-center justify-between gap-2 pt-2.5 border-t border-sp-line/60">
                        <button
                          type="button"
                          data-testid={`btn-preview-plugin-skills-${suite.id}`}
                          onClick={() => setPreviewPluginSuite(suite)}
                          className="text-xs font-medium text-sp-text hover:underline cursor-pointer"
                        >
                          {t("shell.pluginsPage.previewIncludedSkills", { count: suiteSkills.length })}
                        </button>

                        {isInstalled ? (
                          <div className="flex items-center gap-1.5">
                            <button
                              type="button"
                              data-testid={`browse-manage-plugin-btn-${suite.id}`}
                              onClick={() => setPluginFilter("added")}
                              className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-card px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                            >
                              {t("shell.pluginsPage.addedManageLink")}
                            </button>
                            <button
                              type="button"
                              data-testid={`browse-remove-plugin-btn-${suite.id}`}
                              onClick={(e) => handleRemovePlugin(suite, e.currentTarget)}
                              className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-danger-text hover:bg-sp-danger-bg cursor-pointer"
                            >
                              {t("shell.pluginsPage.moveOutBtn")}
                            </button>
                          </div>
                        ) : (
                          <button
                            type="button"
                            data-testid={`browse-install-plugin-btn-${suite.id}`}
                            onClick={() => handleInstallPlugin(suite)}
                            className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-text px-3 text-xs font-medium text-sp-surface hover:opacity-90 cursor-pointer"
                          >
                            <Plus className="h-3.5 w-3.5" strokeWidth={1.75} />
                            <span>{t("shell.pluginsPage.addPluginBtn")}</span>
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

          </div>
        )}
      </main>

      {inspectedConnector && (
        <ModalOverlay
          closeOnBackdropClick={false}
          onClose={() => setInspectedConnectorId(null)}
          role="dialog"
          aria-modal={true}
          aria-labelledby="connector-usage-modal-title"
          data-testid="connector-usage-overlay"
          className="p-4 sm:p-6"
        >
          <div
            data-testid="connector-usage-inspector"
            className="relative flex flex-col w-[min(680px,92vw)] max-h-[82vh] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-hidden animate-in zoom-in-95 duration-150"
          >
            <div className="flex items-start justify-between gap-4 border-b border-sp-line bg-sp-card px-5 py-3.5 shrink-0">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 flex-wrap">
                  <Cable
                    className="h-4 w-4 text-sp-text-secondary shrink-0"
                    strokeWidth={1.75}
                  />
                  <h2
                    id="connector-usage-modal-title"
                    className="text-base font-semibold text-sp-text truncate"
                  >
                    {inspectedConnector.name}
                  </h2>
                  <span className="rounded bg-sp-surface px-2 py-0.5 text-xs text-sp-text-secondary border border-sp-line whitespace-nowrap shrink-0">
                    {inspectedConnector.transport.toUpperCase()}
                  </span>
                </div>
                <p className="mt-1 text-xs text-sp-text-secondary line-clamp-2">
                  {inspectedConnector.description}
                </p>
              </div>

              <div className="flex items-center gap-2 shrink-0">
                <button
                  type="button"
                  data-testid="connector-copy-endpoint-btn"
                  onClick={() => handleCopyConnectorEndpoint(inspectedConnector)}
                  className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap shrink-0 cursor-pointer"
                >
                  {copiedConnectorId === inspectedConnector.id ? (
                    <span className="text-sp-success-text font-medium">
                      {t("shell.pluginsPage.copied")}
                    </span>
                  ) : (
                    <span>{t("shell.pluginsPage.copyEndpointBtn")}</span>
                  )}
                </button>
                <button
                  type="button"
                  data-testid="close-connector-usage-btn"
                  aria-label={t("shell.pluginsPage.closeUsageDetailsAria")}
                  onClick={() => setInspectedConnectorId(null)}
                  className="inline-flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-muted hover:text-sp-text hover:bg-sp-hover shrink-0 cursor-pointer"
                >
                  <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                </button>
              </div>
            </div>

            <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4">
              <div className="rounded-sp-md border border-sp-line bg-sp-card p-3.5 space-y-2">
                <div className="text-xs font-semibold text-sp-text">
                  {t("shell.pluginsPage.endpointConfigHeading")}
                </div>
                <div className="flex items-center gap-2 text-xs text-sp-text-secondary">
                  <span>{t("shell.pluginsPage.transportProtocolLabel")}</span>
                  <code className="rounded bg-sp-surface border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text">
                    {inspectedConnector.transport}
                  </code>
                </div>
                <pre className="overflow-x-auto rounded-sp-sm border border-sp-line bg-sp-surface p-2.5 font-mono text-xs text-sp-text break-all whitespace-pre-wrap">
                  {inspectedConnector.endpoint}
                </pre>
              </div>

              <div className="rounded-sp-md border border-sp-line bg-sp-card p-3.5 space-y-2">
                <div className="text-xs font-semibold text-sp-text">
                  {t("shell.pluginsPage.usageNotesHeading")}
                </div>
                <p className="text-xs text-sp-text-secondary leading-relaxed">
                  {inspectedConnector.transport === "stdio"
                    ? t("shell.pluginsPage.stdioUsageHint")
                    : t("shell.pluginsPage.httpUsageHint")}
                </p>
              </div>
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-sp-line bg-sp-card px-5 py-3 shrink-0">
              <span className="text-xs text-sp-text-muted">
                {inspectedConnector.enabled
                  ? t("shell.pluginsPage.statusEnabled")
                  : t("shell.pluginsPage.stateDisabled")}
              </span>
              <div className="flex items-center gap-3 shrink-0">
                <button
                  type="button"
                  role="switch"
                  aria-checked={inspectedConnector.enabled}
                  aria-label={
                    inspectedConnector.enabled
                      ? t("shell.pluginsPage.disableConnectorAria", { name: inspectedConnector.name })
                      : t("shell.pluginsPage.enableConnectorAria", { name: inspectedConnector.name })
                  }
                  data-state={inspectedConnector.enabled ? "checked" : "unchecked"}
                  data-testid="connector-inspector-toggle-btn"
                  onClick={() => handleToggleCustomConnectorEnabled(inspectedConnector.id)}
                  className="eo-settings-switch shrink-0 outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                >
                  <span className="eo-settings-switch-thumb" />
                </button>
                <button
                  type="button"
                  data-testid="connector-inspector-done-btn"
                  onClick={() => setInspectedConnectorId(null)}
                  className="inline-flex h-8 items-center rounded-sp-sm bg-[#18181b] px-3.5 text-xs font-medium text-white hover:bg-[#27272a] cursor-pointer dark:bg-[#f4f4f5] dark:text-[#18181b]"
                >
                  {t("shell.pluginsPage.doneBtn")}
                </button>
              </div>
            </div>
          </div>
        </ModalOverlay>
      )}

      {inspectedHubSkill && (
        <ModalOverlay
          closeOnBackdropClick={false}
          onClose={() => setInspectedHubSkill(null)}
          role="dialog"
          aria-modal={true}
          aria-labelledby="hub-skill-modal-title"
          data-testid="hub-skill-inspector-overlay"
          className="p-4 sm:p-6"
        >
          {(() => {
            const installed = isHubSkillInstalled(inspectedHubSkill, skills);
            const fullDesc =
              inspectedHubSkill.skill?.description_zh ||
              inspectedHubSkill.description;
            const promptText = inspectedHubSkill.skill?.prompt_template;
            return (
              <div
                data-testid="hub-skill-inspector"
                className="relative flex flex-col w-[min(720px,92vw)] max-h-[84vh] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-hidden animate-in zoom-in-95 duration-150"
              >
                <div className="flex items-start justify-between gap-4 border-b border-sp-line bg-sp-card px-5 py-3.5 shrink-0">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <Puzzle
                        className="h-4 w-4 text-sp-text-secondary shrink-0"
                        strokeWidth={1.75}
                      />
                      <h2
                        id="hub-skill-modal-title"
                        className="text-base font-semibold text-sp-text truncate"
                      >
                        {inspectedHubSkill.name}
                      </h2>
                      <span
                        className={cn(
                          "rounded-full px-2 py-0.5 text-xs font-medium border border-sp-line whitespace-nowrap shrink-0",
                          installed
                            ? "bg-sp-success-bg text-sp-success-text"
                            : "bg-sp-surface text-sp-text-muted",
                        )}
                      >
                        {installed
                          ? t("shell.pluginsPage.skillInstalled")
                          : t("shell.pluginsPage.notInstalledShort")}
                      </span>
                    </div>
                    <p className="mt-1 text-xs text-sp-text-muted">
                      {t("shell.pluginsPage.hubMetaFooter", { version: inspectedHubSkill.version })}
                    </p>
                  </div>

                  <button
                    type="button"
                    data-testid="close-hub-skill-inspector-btn"
                    aria-label={t("shell.pluginsPage.closeHubSkillAria")}
                    onClick={() => setInspectedHubSkill(null)}
                    className="inline-flex h-7 w-7 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-muted hover:text-sp-text hover:bg-sp-hover shrink-0 cursor-pointer"
                  >
                    <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                  </button>
                </div>

                <div className="flex-1 overflow-y-auto px-5 py-4 space-y-4 select-text">
                  <div className="space-y-1.5">
                    <div className="text-xs font-semibold text-sp-text-secondary">
                      {t("shell.pluginsPage.capabilitiesOverview")}
                    </div>
                    <p className="text-xs sm:text-sm text-sp-text leading-relaxed">
                      {fullDesc}
                    </p>
                  </div>

                  {inspectedHubSkill.capabilities &&
                    inspectedHubSkill.capabilities.length > 0 && (
                      <div className="flex flex-wrap items-center gap-1.5">
                        {inspectedHubSkill.capabilities.map((cap) => (
                          <span
                            key={cap}
                            className="rounded border border-sp-line bg-sp-card px-2 py-0.5 text-xs text-sp-text-secondary"
                          >
                            {cap}
                          </span>
                        ))}
                      </div>
                    )}

                  {promptText && (
                    <div className="space-y-1.5">
                      <div className="text-xs font-semibold text-sp-text-secondary">
                        {t("shell.pluginsPage.includedWorkflowPreview")}
                      </div>
                      <pre
                        data-testid="hub-skill-inspector-prompt"
                        className="max-h-64 overflow-y-auto rounded-sp-sm border border-sp-line bg-sp-card p-3 font-mono text-xs text-sp-text leading-relaxed whitespace-pre-wrap"
                      >
                        {promptText}
                      </pre>
                    </div>
                  )}
                </div>

                <div className="flex items-center justify-between gap-3 border-t border-sp-line bg-sp-card px-5 py-3 shrink-0">
                  <span className="text-xs text-sp-text-muted">
                    {installed
                      ? t("shell.pluginsPage.hubSkillInstalledNote")
                      : t("shell.pluginsPage.hubSkillAutoEnableNote")}
                  </span>
                  <div className="flex items-center gap-2 shrink-0">
                    {!installed && (
                      <button
                        type="button"
                        data-testid="hub-skill-inspector-install-btn"
                        onClick={() => {
                          handleInstallSkillFromHub(inspectedHubSkill);
                        }}
                        className="inline-flex h-8 items-center rounded-sp-sm bg-[#18181b] px-3.5 text-xs font-medium text-white hover:bg-[#27272a] cursor-pointer dark:bg-[#f4f4f5] dark:text-[#18181b]"
                      >
                        {t("shell.pluginsPage.installSkillFromHub")}
                      </button>
                    )}
                    <button
                      type="button"
                      data-testid="hub-skill-inspector-done-btn"
                      onClick={() => setInspectedHubSkill(null)}
                      className="inline-flex h-8 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-3.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                    >
                      {t("shell.pluginsPage.doneBtn")}
                    </button>
                  </div>
                </div>
              </div>
            );
          })()}
        </ModalOverlay>
      )}

      {previewPluginSuite && (
        <ModalOverlay
          closeOnBackdropClick={false}
          onClose={() => setPreviewPluginSuite(null)}
          role="dialog"
          aria-modal={true}
          aria-label={t("shell.pluginsPage.previewPluginAria", { name: previewPluginSuite.name })}
          data-testid="plugin-skills-preview-overlay"
          className="p-4 sm:p-6"
        >
          <div
            data-testid="plugin-skills-preview-modal"
            className="relative flex flex-col w-[min(680px,92vw)] max-h-[84vh] rounded-[16px] border border-sp-line bg-sp-surface shadow-sp-modal overflow-hidden"
          >
            <div className="flex items-start justify-between gap-3 border-b border-sp-line px-5 py-4">
              <div className="min-w-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <Package className="h-4 w-4 text-sp-text-secondary shrink-0" strokeWidth={1.75} />
                  <h3 className="text-sm font-semibold text-sp-text">
                    {previewPluginSuite.name}
                  </h3>
                  <span
                    className={cn(
                      "rounded px-2 py-0.5 text-xs font-medium border border-sp-line",
                      installedPluginIds.has(previewPluginSuite.id) &&
                        !disabledPluginIds.has(previewPluginSuite.id)
                        ? "bg-sp-success-bg text-sp-success-text"
                        : "bg-sp-card text-sp-text-secondary",
                    )}
                  >
                    {installedPluginIds.has(previewPluginSuite.id)
                      ? disabledPluginIds.has(previewPluginSuite.id)
                        ? t("shell.pluginsPage.installedDisabledBadge")
                        : t("shell.pluginsPage.installedEnabledBadge")
                      : t("shell.pluginsPage.notInstalledPreviewBadge")}
                  </span>
                </div>
                <p className="mt-1 text-xs text-sp-text-muted">
                  {t("shell.pluginsPage.pluginPreviewMeta", {
                    author: localizeSkillAuthor(previewPluginSuite.author, locale),
                    version: previewPluginSuite.version,
                    count: getSuiteSkills(previewPluginSuite).length,
                  })}
                </p>
              </div>
              <button
                type="button"
                aria-label={t("shell.pluginsPage.closeSkillPreviewAria")}
                data-testid="plugin-skills-preview-close-btn"
                onClick={() => setPreviewPluginSuite(null)}
                className="rounded-sp-sm p-1 text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
              >
                <X className="h-4 w-4" strokeWidth={1.75} />
              </button>
            </div>

            <div className="flex-1 overflow-y-auto px-5 py-4 space-y-3">
              <div className="rounded-sp-sm border border-sp-line bg-sp-card px-3 py-2.5 text-xs text-sp-text-secondary leading-relaxed">
                <span className="font-semibold text-sp-text">
                  {t("shell.pluginsPage.pluginVsSkillNoteTitle")}
                </span>
                {t("shell.pluginsPage.pluginVsSkillNoteBody")}
              </div>

              {(previewPluginSuite.bundledConnectors && previewPluginSuite.bundledConnectors.length > 0
                ? previewPluginSuite.bundledConnectors
                : previewPluginSuite.bundledConnector
                  ? [previewPluginSuite.bundledConnector]
                  : []
              ).map((conn) => (
                <div
                  key={conn.id}
                  data-testid={`plugin-preview-connector-${conn.id}`}
                  className="rounded-sp-sm border border-sp-line bg-sp-card px-3 py-2 text-xs text-sp-text-secondary flex items-center justify-between gap-2 flex-wrap"
                >
                  <span>
                    {t("shell.pluginsPage.companionConnectorLabel")}
                    <strong className="text-sp-text">{conn.name}</strong>
                  </span>
                  <code className="font-mono text-xs text-sp-text-muted">
                    {conn.transport}: {conn.endpoint}
                  </code>
                </div>
              ))}

              {getSuiteSkills(previewPluginSuite).map((sk) => (
                <div
                  key={sk.id}
                  data-testid={`plugin-preview-skill-item-${sk.id}`}
                  className="rounded-sp-md border border-sp-line bg-sp-card p-3.5 space-y-2"
                >
                  <div className="flex items-center justify-between gap-2 flex-wrap">
                    <div className="flex items-center gap-2">
                      <FileText className="h-3.5 w-3.5 text-sp-text-secondary shrink-0" strokeWidth={1.75} />
                      <span className="text-xs font-semibold text-sp-text">
                        {sk.name}
                      </span>
                    </div>
                    {sk.slashCommand && (
                      <code className="rounded bg-sp-surface border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text-muted">
                        {sk.slashCommand}
                      </code>
                    )}
                  </div>
                  <p className="text-xs text-sp-text-secondary">
                    {sk.description}
                  </p>
                  <pre className="max-h-60 overflow-y-auto rounded-sp-sm border border-sp-line bg-sp-surface p-3 font-mono text-xs text-sp-text whitespace-pre-wrap break-words leading-relaxed">
                    {sk.prompt}
                  </pre>
                </div>
              ))}
            </div>

            <div className="flex items-center justify-between gap-2 border-t border-sp-line px-5 py-3 bg-sp-card/60">
              <button
                type="button"
                data-testid="plugin-preview-goto-skills-btn"
                onClick={() => {
                  const disc = previewPluginSuite.discipline;
                  setPreviewPluginSuite(null);
                  setSkillFilter(disc);
                  setActiveTab("skills");
                }}
                className="text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:underline cursor-pointer"
              >
                {t("shell.pluginsPage.viewAllIndependentSkills")}
              </button>
              <div className="flex items-center gap-2">
                {!installedPluginIds.has(previewPluginSuite.id) && (
                  <button
                    type="button"
                    data-testid="plugin-preview-install-btn"
                    onClick={() => {
                      const suite = previewPluginSuite;
                      setPreviewPluginSuite(null);
                      handleInstallPlugin(suite);
                    }}
                    className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-text px-3 text-xs font-medium text-sp-surface hover:opacity-90 cursor-pointer"
                  >
                    <Plus className="h-3.5 w-3.5" strokeWidth={1.75} />
                    <span>{t("shell.pluginsPage.addThisPluginBtn")}</span>
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => setPreviewPluginSuite(null)}
                  className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                >
                  {t("shell.pluginsPage.closeBtn")}
                </button>
              </div>
            </div>
          </div>
        </ModalOverlay>
      )}

      <Popconfirm
        open={pendingPopconfirm !== null}
        anchorEl={pendingPopconfirm?.anchorEl}
        title={pendingPopconfirm?.title ?? ""}
        description={pendingPopconfirm?.description}
        confirmLabel={pendingPopconfirm?.confirmLabel ?? t("shell.pluginsPage.confirmFallback")}
        cancelLabel={t("shell.pluginsPage.cancel")}
        tone="danger"
        onCancel={() => setPendingPopconfirm(null)}
        onConfirm={() => {
          const current = pendingPopconfirm;
          setPendingPopconfirm(null);
          if (!current) return;
          if (current.kind === "skill") {
            performDeleteSkill(current.id);
          } else if (current.kind === "connector") {
            performDeleteCustomConnector(current.id);
          } else if (current.kind === "plugin" && current.suite) {
            performRemovePlugin(current.suite);
          } else if (current.kind === "discard-skill-form") {
            closeSkillDrawerNow();
          }
        }}
      />
    </div>
  );
}

const CURATED_CONDA_GEO_PACKAGES: ReadonlyArray<{
  id: string;
  label: string;
  summary: string;
  commandPreview: string;
}> = [
  {
    id: "isce2",
    label: "ISCE2",
    summary: "NASA/JPL 雷达干涉核心处理框架（Sentinel-1 TOPS / NISAR）",
    commandPreview: "conda install -y -c conda-forge isce2",
  },
  {
    id: "mintpy",
    label: "MintPy",
    summary: "SBAS / PS-InSAR 时序形变反演与 GACOS 大气延迟改正",
    commandPreview: "conda install -y -c conda-forge mintpy",
  },
  {
    id: "xdem",
    label: "xDEM",
    summary: "三维 DEM 地形配准、冰川物质平衡与高程变化误差评估",
    commandPreview: "conda install -y -c conda-forge xdem",
  },
  {
    id: "pytmd",
    label: "pyTMD",
    summary: "TUM EOT20 / FES 全球海洋潮汐调和分析与海岸潮汐负荷改正",
    commandPreview: "conda install -y -c conda-forge pytmd",
  },
  {
    id: "torchgeo",
    label: "TorchGeo",
    summary: "遥感时空瓦片采样器与 Prithvi / DOFA 地球观测基础模型库",
    commandPreview: "conda install -y -c conda-forge torchgeo",
  },
];

function detectClientPlatformTag(hostPlatform?: string | null): string {
  const trimmedHost = (hostPlatform || "").trim();
  if (trimmedHost) return trimmedHost;
  const gProcess = (
    globalThis as { process?: { platform?: string; arch?: string } }
  ).process;
  const nav = typeof navigator !== "undefined" ? navigator : undefined;
  const navPlat = String(
    (nav as { userAgentData?: { platform?: string } } | undefined)
      ?.userAgentData?.platform ||
      nav?.platform ||
      nav?.userAgent ||
      "",
  ).toLowerCase();
  const plat = gProcess?.platform || navPlat;
  const arch = (gProcess?.arch || "").toLowerCase();
  if (plat === "win32" || navPlat.includes("win")) {
    return arch === "arm64" ? "windows-aarch64" : "windows-x86_64";
  }
  if (plat === "darwin" || navPlat.includes("mac")) {
    if (arch === "x64") return "macos-x86_64";
    if (arch === "arm64") return "macos-aarch64";
    return navPlat.includes("intel") ? "macos-x86_64" : "macos-aarch64";
  }
  if (plat === "linux" || navPlat.includes("linux") || navPlat.includes("x11")) {
    return arch === "arm64" || navPlat.includes("aarch64")
      ? "linux-aarch64"
      : "linux-x86_64";
  }
  return "windows-x86_64";
}

function runtimeComponentName(
  compId: string,
  fallback: string,
  tr: ReturnType<typeof useI18n>["t"],
): string {
  switch (compId) {
    case "eostage-core":
      return tr("shell.pluginsPage.runtimeNames.eostage-core");
    case "native-gis-runtime":
      return tr("shell.pluginsPage.runtimeNames.native-gis-runtime");
    case "vertical-grid-egm96":
      return tr("shell.pluginsPage.runtimeNames.vertical-grid-egm96");
    case "vertical-grid-egm2008":
      return tr("shell.pluginsPage.runtimeNames.vertical-grid-egm2008");
    case "compat-worker":
      return tr("shell.pluginsPage.runtimeNames.compat-worker");
    case "whitebox-cli":
      return tr("shell.pluginsPage.runtimeNames.whitebox-cli");
    case "duckdb-cli":
      return tr("shell.pluginsPage.runtimeNames.duckdb-cli");
    case "rustgis-cli":
      return tr("shell.pluginsPage.runtimeNames.rustgis-cli");
    default:
      return fallback;
  }
}

function runtimeCapabilitySummary(
  compId: string,
  fallback: string,
  tr: ReturnType<typeof useI18n>["t"],
): string {
  switch (compId) {
    case "eostage-core":
      return tr("shell.pluginsPage.runtimeCapabilities.eostage-core");
    case "native-gis-runtime":
      return tr("shell.pluginsPage.runtimeCapabilities.native-gis-runtime");
    case "vertical-grid-egm96":
      return tr("shell.pluginsPage.runtimeCapabilities.vertical-grid-egm96");
    case "vertical-grid-egm2008":
      return tr("shell.pluginsPage.runtimeCapabilities.vertical-grid-egm2008");
    case "compat-worker":
      return tr("shell.pluginsPage.runtimeCapabilities.compat-worker");
    case "whitebox-cli":
      return tr("shell.pluginsPage.runtimeCapabilities.whitebox-cli");
    case "duckdb-cli":
      return tr("shell.pluginsPage.runtimeCapabilities.duckdb-cli");
    case "rustgis-cli":
      return tr("shell.pluginsPage.runtimeCapabilities.rustgis-cli");
    default:
      return fallback;
  }
}

function externalGisCapabilitySummary(
  toolId: string,
  fallback: string | undefined,
  tr: ReturnType<typeof useI18n>["t"],
): string | undefined {
  switch (toolId) {
    case "qgis":
      return tr("shell.pluginsPage.externalGisCapabilities.qgis");
    case "geolibre":
      return tr("shell.pluginsPage.externalGisCapabilities.geolibre");
    case "rustgis":
      return tr("shell.pluginsPage.externalGisCapabilities.rustgis");
    case "arcgis_pro":
      return tr("shell.pluginsPage.externalGisCapabilities.arcgis_pro");
    default:
      return fallback;
  }
}

function condaPackageSummary(
  pkgId: string,
  fallback: string,
  tr: ReturnType<typeof useI18n>["t"],
): string {
  switch (pkgId) {
    case "isce2":
      return tr("shell.pluginsPage.condaPackages.isce2");
    case "mintpy":
      return tr("shell.pluginsPage.condaPackages.mintpy");
    case "xdem":
      return tr("shell.pluginsPage.condaPackages.xdem");
    case "pytmd":
      return tr("shell.pluginsPage.condaPackages.pytmd");
    case "torchgeo":
      return tr("shell.pluginsPage.condaPackages.torchgeo");
    default:
      return fallback;
  }
}

function formatPlatformSupportBadge(
  platforms: string[] | null | undefined,
  componentId: string | undefined,
  tr: ReturnType<typeof useI18n>["t"],
): string {
  const list = Array.isArray(platforms)
    ? platforms.map((p) => p.trim()).filter(Boolean)
    : [];
  const effective =
    list.length > 0
      ? list
      : componentId === "compat-worker"
        ? ["windows-x86_64"]
        : [];
  if (effective.length === 0) return tr("shell.pluginsPage.platformAll");
  const hasWin = effective.some((p) => p.startsWith("windows-"));
  const hasMac = effective.some((p) => p.startsWith("macos-"));
  const hasLinux = effective.some((p) => p.startsWith("linux-"));
  if (hasWin && hasMac && hasLinux) return tr("shell.pluginsPage.platformAll");
  if (hasWin && hasMac) {
    const macOnlyArm =
      effective.filter((p) => p.startsWith("macos-")).every((p) => p === "macos-aarch64");
    return macOnlyArm
      ? tr("shell.pluginsPage.platformWinMacArm")
      : tr("shell.pluginsPage.platformWinMac");
  }
  if (hasWin && !hasMac && !hasLinux) return tr("shell.pluginsPage.platformWinOnly");
  if (effective.length === 1 && effective[0] === "macos-aarch64") {
    return tr("shell.pluginsPage.platformMacArmOnly");
  }
  if (hasMac && !hasWin && !hasLinux) return tr("shell.pluginsPage.platformMacOnly");
  return effective.join(" / ");
}

function resolveHostMirrorPackage(
  entry: ExtensionRegistryEntry,
  hostTag: string,
  tr: ReturnType<typeof useI18n>["t"],
): {
  supported: boolean;
  available: boolean;
  packageUrl: string;
  sha256: string;
  size: number;
  disabledReason?: string;
} {
  const platforms = Array.isArray(entry.platforms)
    ? entry.platforms.map((p) => p.trim()).filter(Boolean)
    : entry.id === "compat-worker"
      ? ["windows-x86_64"]
      : [];
  const hostInPlatforms =
    platforms.length === 0 || platforms.includes(hostTag);
  const platformPkgs = entry.platform_packages;
  const hasPlatformPkgMap =
    Boolean(platformPkgs) && Object.keys(platformPkgs || {}).length > 0;
  const hostPkg = platformPkgs?.[hostTag];

  if (!hostInPlatforms) {
    const winOnlyComponent =
      entry.id === "compat-worker" ||
      (platforms.length > 0 && platforms.every((p) => p.startsWith("windows-")));
    const isGenuineNonApplicable =
      entry.id === "compat-worker" ||
      (winOnlyComponent && entry.id !== "native-gis-runtime");
    return {
      supported: false,
      available: false,
      packageUrl: "",
      sha256: "",
      size: 0,
      disabledReason: isGenuineNonApplicable
        ? tr("shell.pluginsPage.packageNotRequiredOs")
        : tr("shell.pluginsPage.packagePendingRelease"),
    };
  }

  if (hasPlatformPkgMap) {
    if (hostPkg && hostPkg.package.trim()) {
      return {
        supported: true,
        available: true,
        packageUrl: hostPkg.package.trim(),
        sha256: hostPkg.sha256 || entry.sha256,
        size: hostPkg.size ?? entry.size,
      };
    }
    return {
      supported: true,
      available: false,
      packageUrl: "",
      sha256: "",
      size: 0,
      disabledReason: tr("shell.pluginsPage.packagePendingRelease"),
    };
  }

  if (entry.package && entry.package.trim()) {
    return {
      supported: true,
      available: true,
      packageUrl: entry.package.trim(),
      sha256: entry.sha256,
      size: entry.size,
    };
  }
  return {
    supported: true,
    available: false,
    packageUrl: "",
    sha256: "",
    size: 0,
    disabledReason: tr("shell.pluginsPage.packagePendingRelease"),
  };
}

/**
 * 设置中心 -> 「运行环境与工具」独立面板
 * 统一承载本机 GDAL / PROJ 共享运行组件、外部 GIS 工具链与科学计算环境。
 */
export function RuntimeToolsSettingsSection({
  fetchRegistryOverride,
  getRuntimeComponentsOverride,
}: RuntimeToolsSettingsSectionProps) {
  const { t } = useI18n();
  const persistedMirrorInit = useMemo(() => loadPersistedExtensionMirror(), []);
  const registryUrlRef = useRef<string>(persistedMirrorInit.url);
  const [registryReport, setRegistryReport] =
    useState<ExtensionRegistryReport | null>(null);
  const [registryLoading, setRegistryLoading] = useState(false);
  const [registryError, setRegistryError] = useState<string>("");

  const [runtimeReport, setRuntimeReport] =
    useState<RuntimeComponentsReport | null>(null);
  const [runtimeLoading, setRuntimeLoading] = useState(false);
  const [installingComponentId, setInstallingComponentId] = useState<
    string | null
  >(null);
  const [localInstallOpenId, setLocalInstallOpenId] = useState<string | null>(
    null,
  );
  const [localSourceInput, setLocalSourceInput] = useState<string>("");
  const [localSha256Input, setLocalSha256Input] = useState<string>("");
  const [runtimeBanner, setRuntimeBanner] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);

  const [externalGisReport, setExternalGisReport] =
    useState<ExternalGisInspectReport | null>(null);
  const [externalGisLoading, setExternalGisLoading] = useState(false);
  const [externalGisPaths, setExternalGisPaths] = useState<
    Record<string, string>
  >({});
  const [rtkCustomPath, setRtkCustomPath] = useState<string>("");
  const [condaAliasInput, setCondaAliasInput] = useState<string>("");
  const [condaPathInput, setCondaPathInput] = useState<string>("");
  const [installingCondaPkgId, setInstallingCondaPkgId] = useState<string | null>(
    null,
  );
  const [pendingCondaInstall, setPendingCondaInstall] = useState<{
    pkgId: string;
    label: string;
    commandPreview: string;
    targetEnv: string | null;
    anchorEl: HTMLElement | null;
  } | null>(null);
  const [externalGisBanner, setExternalGisBanner] = useState<{
    type: "success" | "error";
    text: string;
  } | null>(null);
  const [customConnectors, setCustomConnectors] = useState<
    CustomConnectorItem[]
  >(loadCustomConnectors);

  const { spinClass: runtimeSpinClass } = useFullTurnSpin(runtimeLoading);
  const showInstallingSpinner = useDelayedFlag(Boolean(installingComponentId));

  const fetchRuntimeComponents = useCallback(async (manual = false) => {
    setRuntimeLoading(true);
    try {
      const fetcher = getRuntimeComponentsOverride ?? getRuntimeComponents;
      const report = await fetcher();
      setRuntimeReport(report);
      if (manual) {
        emitTopToast({ text: t("shell.pluginsPage.runtimeToastChecked"), tone: "success" });
      }
    } catch {
      if (manual) {
        emitTopToast({ text: t("shell.pluginsPage.runtimeToastCheckFailed"), tone: "danger" });
      }
    } finally {
      setRuntimeLoading(false);
    }
  }, [getRuntimeComponentsOverride, t]);

  const handleLoadRegistryRef = useRef<
    ((overrideUrl?: string, manual?: boolean) => Promise<void>) | null
  >(null);
  const externalGisPrefsRef = useRef(externalGisReport?.prefs);
  externalGisPrefsRef.current = externalGisReport?.prefs;

  const persistMirrorSelection = useCallback(
    async (presetId: string, url: string) => {
      const normalized = normalizeExtensionMirrorSelection(presetId, url);
      savePersistedExtensionMirror(normalized.presetId, normalized.url);
      if (isTauriDesktopHost()) {
        const cur =
          externalGisPrefsRef.current ??
          (await inspectExternalGis(false).catch(() => null))?.prefs;
        if (cur) {
          const payload: ExternalGisPrefs = {
            custom_paths: { ...(cur.custom_paths || {}) },
            preferred_toolchain: cur.preferred_toolchain ?? null,
            conda_custom_envs: { ...(cur.conda_custom_envs || {}) },
            preferred_conda_env: cur.preferred_conda_env ?? null,
            rtk_custom_path: cur.rtk_custom_path ?? null,
            dehydrator_mode: cur.dehydrator_mode ?? "auto",
            extension_mirror_preset: normalized.presetId,
            extension_registry_url: normalized.url,
          };
          try {
            const updated = await saveExternalGisPrefs(payload);
            if (updated) {
              setExternalGisReport(updated);
            }
          } catch {
            // 忽略同步保存异常，localStorage 已落盘
          }
        }
      }
      return normalized;
    },
    [],
  );

  const fetchExternalGis = useCallback(async (probeVersions = false) => {
    setExternalGisLoading(true);
    try {
      const report = await inspectExternalGis(probeVersions);
      if (report) {
        setExternalGisReport(report);
        setExternalGisPaths((prev) => ({
          ...prev,
          ...(report.prefs?.custom_paths || {}),
        }));
        setRtkCustomPath(report.prefs?.rtk_custom_path || "");
        if (
          report.prefs?.extension_mirror_preset ||
          report.prefs?.extension_registry_url
        ) {
          const hadLegacyCloudflare =
            report.prefs.extension_mirror_preset === "cloudflare" ||
            (report.prefs.extension_registry_url || "")
              .toLowerCase()
              .includes("eostage-hub.pages.dev");
          const normalized = normalizeExtensionMirrorSelection(
            report.prefs.extension_mirror_preset,
            report.prefs.extension_registry_url,
          );
          const changed = normalized.url !== registryUrlRef.current;
          registryUrlRef.current = normalized.url;
          savePersistedExtensionMirror(normalized.presetId, normalized.url);
          if (hadLegacyCloudflare) {
            void persistMirrorSelection(normalized.presetId, normalized.url);
          }
          if (changed) {
            void handleLoadRegistryRef.current?.(normalized.url, false);
          }
        }
      }
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.message || String(err || t("shell.pluginsPage.externalGisProbeFailed")),
      });
    } finally {
      setExternalGisLoading(false);
    }
  }, [persistMirrorSelection, t]);

  const handleLoadRegistry = useCallback(
    async (overrideUrl?: string, manual = false) => {
      const targetUrl =
        overrideUrl !== undefined ? overrideUrl : registryUrlRef.current.trim();
      const fetcher = fetchRegistryOverride ?? fetchExtensionRegistry;
      setRegistryLoading(true);
      setRegistryError("");
      const lwId = manual
        ? startLongWork({
            title: t("shell.pluginsPage.registryLoadingTitle"),
            category: "plugin",
            stageLabel: t("shell.pluginsPage.registryLoadingStage"),
          })
        : null;
      try {
        const report = await fetcher(targetUrl ? targetUrl : undefined);
        setRegistryReport(report);
        if (manual) {
          emitTopToast({ text: t("shell.pluginsPage.registryUpdatedToast"), tone: "success" });
          if (lwId) {
            completeLongWork(lwId, {
              doneTitle: t("shell.pluginsPage.registryUpdatedToast"),
              doneSubtitle: t("shell.pluginsPage.registryUpdatedSubtitle", {
                count: report?.entries?.length ?? 0,
              }),
            });
          }
        }
      } catch (err: any) {
        setRegistryReport(null);
        const msg =
          err?.context || err?.message || String(err || t("shell.pluginsPage.registryLoadFailed"));
        setRegistryError(msg);
        if (lwId) {
          failLongWork(lwId, msg);
        }
      } finally {
        setRegistryLoading(false);
      }
    },
    [fetchRegistryOverride, t],
  );
  handleLoadRegistryRef.current = handleLoadRegistry;

  const autoLoadedRef = useRef(false);
  useEffect(() => {
    if (autoLoadedRef.current) return;
    autoLoadedRef.current = true;
    void fetchRuntimeComponents(false);
    void fetchExternalGis(false);
    void handleLoadRegistry();
    void getPluginsStatus()
      .then((status) => {
        if (!status || !Array.isArray(status.custom_connectors)) return;
        setCustomConnectors((prev) => {
          const byId = new Map<string, CustomConnectorItem>();
          for (const item of status.custom_connectors) {
            byId.set(item.id, {
              id: item.id,
              name: item.name,
              transport: item.transport === "stdio" ? "stdio" : "http",
              endpoint: item.endpoint,
              description: item.description,
              enabled: item.enabled,
            });
          }
          for (const item of prev) {
            byId.set(item.id, item);
          }
          return Array.from(byId.values());
        });
      })
      .catch(() => {});
  }, [fetchRuntimeComponents, fetchExternalGis, handleLoadRegistry]);

  const sortedRuntimeComponents = useMemo(() => {
    const list = runtimeReport?.components || [];
    return [...list].sort((a, b) => {
      const aOrder = RUNTIME_COMPONENT_META[a.id]?.sortOrder ?? 50;
      const bOrder = RUNTIME_COMPONENT_META[b.id]?.sortOrder ?? 50;
      return aOrder - bOrder;
    });
  }, [runtimeReport?.components]);

  const hostPlatformTag = useMemo(
    () => detectClientPlatformTag(runtimeReport?.host_platform),
    [runtimeReport?.host_platform],
  );

  const componentRegistryMap = useMemo(() => {
    const map = new Map<string, ExtensionRegistryEntry>();
    for (const entry of registryReport?.entries || []) {
      if (entry.kind === "component") {
        map.set(entry.id, entry);
      }
    }
    return map;
  }, [registryReport]);

  const handleInstallFromMirror = async (
    compId: string,
    entry: ExtensionRegistryEntry,
  ) => {
    const resolved = resolveHostMirrorPackage(entry, hostPlatformTag, t);
    if (!resolved.available || !resolved.packageUrl) {
      setRuntimeBanner({
        type: "error",
        text: resolved.disabledReason || t("shell.pluginsPage.noMirrorPackage"),
      });
      return;
    }
    setInstallingComponentId(compId);
    setRuntimeBanner(null);
    const lwId = startLongWork({
      title: t("shell.pluginsPage.installCompTitle", { id: compId }),
      category: "plugin",
      stageLabel: t("shell.pluginsPage.installCompStage", { id: compId, version: entry.version }),
    });
    try {
      const res = await installRuntimeComponent({
        component_id: compId,
        source: resolved.packageUrl,
        expected_sha256: resolved.sha256,
      });
      await fetchRuntimeComponents();
      setRuntimeBanner({
        type: "success",
        text: t("shell.pluginsPage.installCompSuccess", {
          id: compId,
          version: res.version,
          sha: res.sha256.slice(0, 12),
          detail: res.detail,
        }),
      });
      completeLongWork(lwId, {
        doneTitle: t("shell.pluginsPage.installCompDoneTitle", { id: compId }),
        doneSubtitle: t("shell.pluginsPage.installCompDoneSubtitle", { version: res.version }),
      });
    } catch (err: any) {
      const msg =
        err?.context ||
        err?.message ||
        String(err || t("shell.pluginsPage.installCompFailed", { id: compId }));
      setRuntimeBanner({
        type: "error",
        text: msg,
      });
      failLongWork(lwId, msg);
    } finally {
      setInstallingComponentId(null);
    }
  };

  const handleInstallFromLocal = async (
    e: React.FormEvent,
    compId: string,
  ) => {
    e.preventDefault();
    const src = localSourceInput.trim();
    if (!src) {
      setRuntimeBanner({
        type: "error",
        text: t("shell.pluginsPage.localPathRequired"),
      });
      return;
    }
    setInstallingComponentId(compId);
    setRuntimeBanner(null);
    const lwId = startLongWork({
      title: t("shell.pluginsPage.localInstallTitle", { id: compId }),
      category: "plugin",
      stageLabel: t("shell.pluginsPage.localInstallStage"),
    });
    try {
      const res = await installRuntimeComponent({
        component_id: compId,
        source: src,
        expected_sha256: localSha256Input.trim() || undefined,
      });
      setLocalInstallOpenId(null);
      setLocalSourceInput("");
      setLocalSha256Input("");
      await fetchRuntimeComponents();
      setRuntimeBanner({
        type: "success",
        text: t("shell.pluginsPage.localInstallSuccess", {
          id: compId,
          sha: res.sha256.slice(0, 12),
          detail: res.detail,
        }),
      });
      completeLongWork(lwId, {
        doneTitle: t("shell.pluginsPage.localInstallDoneTitle", { id: compId }),
        doneSubtitle: t("shell.pluginsPage.localInstallDoneSubtitle"),
      });
    } catch (err: any) {
      const msg =
        err?.context ||
        err?.message ||
        String(err || t("shell.pluginsPage.localInstallFailed", { id: compId }));
      setRuntimeBanner({
        type: "error",
        text: msg,
      });
      failLongWork(lwId, msg);
    } finally {
      setInstallingComponentId(null);
    }
  };

  const persistExternalGisPrefs = async (
    patch: Partial<ExternalGisPrefs>,
    successText: string,
  ) => {
    setExternalGisBanner(null);
    const cur = externalGisReport?.prefs;
    const payload: ExternalGisPrefs = {
      custom_paths: patch.custom_paths ?? { ...(cur?.custom_paths || {}) },
      preferred_toolchain:
        patch.preferred_toolchain !== undefined
          ? patch.preferred_toolchain
          : (cur?.preferred_toolchain ?? null),
      conda_custom_envs:
        patch.conda_custom_envs ?? { ...(cur?.conda_custom_envs || {}) },
      preferred_conda_env:
        patch.preferred_conda_env !== undefined
          ? patch.preferred_conda_env
          : (cur?.preferred_conda_env ?? null),
      rtk_custom_path:
        patch.rtk_custom_path !== undefined
          ? patch.rtk_custom_path
          : (cur?.rtk_custom_path ?? null),
      dehydrator_mode:
        patch.dehydrator_mode !== undefined
          ? patch.dehydrator_mode
          : (cur?.dehydrator_mode ?? "auto"),
      ...(patch.extension_mirror_preset !== undefined ||
      cur?.extension_mirror_preset !== undefined
        ? {
            extension_mirror_preset:
              patch.extension_mirror_preset !== undefined
                ? patch.extension_mirror_preset
                : cur?.extension_mirror_preset,
          }
        : {}),
      ...(patch.extension_registry_url !== undefined ||
      cur?.extension_registry_url !== undefined
        ? {
            extension_registry_url:
              patch.extension_registry_url !== undefined
                ? patch.extension_registry_url
                : cur?.extension_registry_url,
          }
        : {}),
    };
    const updated = await saveExternalGisPrefs(payload);
    if (updated) {
      setExternalGisReport(updated);
      setExternalGisPaths(updated.prefs?.custom_paths || {});
      setRtkCustomPath(updated.prefs?.rtk_custom_path || "");
      setExternalGisBanner({ type: "success", text: successText });
    }
    return updated;
  };

  const handleSaveExternalGisPath = async (toolchainId: string) => {
    const mergedPaths: Record<string, string> = {
      ...(externalGisReport?.prefs?.custom_paths || {}),
      ...externalGisPaths,
      [toolchainId]: (externalGisPaths[toolchainId] || "").trim(),
    };
    try {
      await persistExternalGisPrefs(
        { custom_paths: mergedPaths },
        `${toolchainId}: ${t("shell.pluginsPage.externalGisSavePath")}`,
      );
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.savePathFailed")),
      });
    }
  };

  const handleSetDehydratorMode = async (mode: "auto" | "builtin_only") => {
    try {
      await persistExternalGisPrefs(
        { dehydrator_mode: mode },
        mode === "auto"
          ? t("shell.pluginsPage.logDehydratorModeAuto")
          : t("shell.pluginsPage.logDehydratorModeBuiltin"),
      );
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.saveDehydratorFailed")),
      });
    }
  };

  const handleSaveRtkPath = async () => {
    const trimmed = rtkCustomPath.trim();
    try {
      await persistExternalGisPrefs(
        { rtk_custom_path: trimmed || null },
        t("shell.pluginsPage.sharedRtkSavePath"),
      );
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.saveRtkFailed")),
      });
    }
  };

  const handleSetPreferredCondaEnv = async (envName: string) => {
    try {
      await persistExternalGisPrefs(
        { preferred_conda_env: envName },
        `${envName}: ${t("shell.pluginsPage.condaDefaultBadge")}`,
      );
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.setCondaFailed")),
      });
    }
  };

  const handleAddCustomCondaEnv = async () => {
    const alias = condaAliasInput.trim();
    const envPath = condaPathInput.trim();
    if (!alias || !envPath) return;
    const mergedEnvs: Record<string, string> = {
      ...(externalGisReport?.prefs?.conda_custom_envs || {}),
      [alias]: envPath,
    };
    try {
      const updated = await persistExternalGisPrefs(
        { conda_custom_envs: mergedEnvs },
        `${alias}: ${t("shell.pluginsPage.condaAddCustomEnv")}`,
      );
      if (updated) {
        setCondaAliasInput("");
        setCondaPathInput("");
      }
    } catch (err: any) {
      setExternalGisBanner({
        type: "error",
        text: err?.context || err?.message || String(err || t("shell.pluginsPage.addCondaFailed")),
      });
    }
  };

  const handleRegisterExternalGisMcp = (item: ExternalGisToolchainInfo) => {
    const exePath = (item.executable_path || item.id).trim();
    const quotedExe = /\s/.test(exePath) ? `"${exePath}"` : exePath;
    const mcpArgs = item.id === "rustgis" ? "mcp --yes" : "--mcp";
    const connectorId =
      item.id === "rustgis" ? "plugin-mcp-rustgis" : `external-mcp-${item.id}`;
    const nextItem: CustomConnectorItem = {
      id: connectorId,
      name: `${item.display_name} MCP (stdio)`,
      transport: "stdio",
      endpoint: `${quotedExe} ${mcpArgs}`,
      description: t("shell.pluginsPage.externalGisMcpDesc", { name: item.display_name }),
      enabled: true,
    };
    const existingIdx = customConnectors.findIndex((c) => c.id === connectorId);
    const updated =
      existingIdx >= 0
        ? customConnectors.map((c, idx) => (idx === existingIdx ? nextItem : c))
        : [...customConnectors, nextItem];
    setCustomConnectors(updated);
    saveCustomConnectors(updated);
    void saveCustomConnectorBackend(nextItem).catch(() => {});
    setExternalGisBanner({
      type: "success",
      text: `${item.display_name}: ${t("shell.pluginsPage.externalGisMcpRegistered")}`,
    });
  };

  return (
    <div data-testid="runtime-panel" className="space-y-4 select-text">
      <section aria-label={t("shell.pluginsPage.runtimeTitle")} className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-sp-md border border-sp-line bg-sp-surface p-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-sm font-semibold text-sp-text">
            {t("shell.pluginsPage.runtimeTitle")}
          </h3>
          <p className="mt-0.5 text-xs text-sp-text-secondary leading-relaxed">
            {t("shell.pluginsPage.runtimeSectionDesc")}
          </p>
          {runtimeReport && (
            <p className="mt-1 font-mono text-xs text-sp-text-muted break-all">
              {t("shell.pluginsPage.installPathLine", { path: runtimeReport.install_root })}
            </p>
          )}
        </div>

        <button
          type="button"
          onClick={() => {
            void fetchRuntimeComponents(true);
            if (registryError || !registryReport) {
              void handleLoadRegistry(undefined, false);
            }
          }}
          disabled={runtimeLoading}
          aria-busy={runtimeLoading}
          data-testid="recheck-runtime-btn"
          className="inline-flex h-8 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors whitespace-nowrap cursor-pointer shrink-0 disabled:opacity-50"
        >
          <RefreshCw className={cn("h-3.5 w-3.5 shrink-0", runtimeSpinClass)} strokeWidth={1.75} />
          <span>{t("shell.pluginsPage.recheck")}</span>
        </button>
      </div>

      {registryError && (
        <EmptyState
          kind="failed"
          size="card"
          testId="registry-error-text"
          visual={<EmptyStateIcon name="wifi-off" size={22} />}
          title={t("shell.pluginsPage.registryOfflineTitle")}
          primaryAction={{
            label: t("shell.pluginsPage.retry"),
            onClick: () => void handleLoadRegistry(undefined, true),
            loading: registryLoading,
          }}
        />
      )}

      {runtimeBanner && (
        <div
          data-testid="runtime-install-banner"
          className={cn(
            "flex items-center justify-between gap-2 rounded-sp-sm border px-3 py-2.5 text-xs",
            runtimeBanner.type === "success"
              ? "bg-sp-success-bg text-sp-success-text border-sp-line"
              : "bg-sp-danger-bg text-sp-danger-text border-sp-line",
          )}
        >
          <span>{runtimeBanner.text}</span>
          <button
            type="button"
            onClick={() => setRuntimeBanner(null)}
            className="text-current opacity-70 hover:opacity-100 cursor-pointer"
          >
            <X className="h-3.5 w-3.5" strokeWidth={1.75} />
          </button>
        </div>
      )}
      </section>

      <section aria-label={t("shell.pluginsPage.runtimeListAria")}>
      <div className="grid grid-cols-1 gap-3.5 pt-1">
        {sortedRuntimeComponents.map((comp) => {
          const isReady = comp.status === "ready";
          const isPartial = comp.status === "partial";
          const meta = RUNTIME_COMPONENT_META[comp.id];
          const isUnderDev = Boolean(meta?.underDevelopment && !isReady);
          const canInstall = comp.id !== "eostage-core" && !isUnderDev;
          const mirrorEntry = componentRegistryMap.get(comp.id);
          const isInstalling = installingComponentId === comp.id;
          const isLocalPanelOpen = localInstallOpenId === comp.id;
          const capabilityLine = runtimeCapabilitySummary(
            comp.id,
            meta?.capabilitySummary || comp.detail,
            t,
          );

          return (
            <div
              key={comp.id}
              data-testid={`runtime-card-${comp.id}`}
              className="rounded-sp-md border border-sp-line bg-sp-surface p-4"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center justify-between gap-2 flex-wrap">
                    <div className="flex items-center gap-2 flex-wrap">
                      <h4 className="text-sm font-semibold text-sp-text truncate">
                        {runtimeComponentName(comp.id, comp.name, t)}
                      </h4>

                      {(mirrorEntry || comp.id === "compat-worker") && !isUnderDev && (
                        <span
                          data-testid={`runtime-platform-badge-${comp.id}`}
                          className="rounded border border-sp-line bg-sp-card px-1.5 py-0.5 text-xs text-sp-text-muted whitespace-nowrap"
                        >
                          {formatPlatformSupportBadge(
                            mirrorEntry?.platforms,
                            comp.id,
                            t,
                          )}
                        </span>
                      )}

                      <span
                        data-testid={`runtime-status-badge-${comp.id}`}
                        className={cn(
                          "inline-flex items-center gap-1 rounded-full border border-sp-line px-2 py-0.5 text-xs font-medium whitespace-nowrap",
                          isUnderDev
                            ? "bg-sp-card text-sp-text-muted"
                            : isReady
                              ? "bg-sp-success-bg text-sp-success-text"
                              : isPartial
                                ? "bg-sp-card text-sp-warning-text"
                                : "bg-sp-card text-sp-text-muted",
                        )}
                      >
                        {isUnderDev ? (
                          <Clock className="h-3 w-3" strokeWidth={1.75} />
                        ) : isReady ? (
                          <CheckCircle2 className="h-3 w-3" strokeWidth={1.75} />
                        ) : isPartial ? (
                          <AlertTriangle className="h-3 w-3" strokeWidth={1.75} />
                        ) : (
                          <XCircle className="h-3 w-3" strokeWidth={1.75} />
                        )}
                        <span>
                          {isUnderDev
                            ? t("shell.pluginsPage.statusUnderDev")
                            : isReady
                              ? t("shell.pluginsPage.statusReady")
                              : isPartial
                                ? t("shell.pluginsPage.statusPartial")
                                : t("shell.pluginsPage.statusNotInstalled")}
                        </span>
                      </span>
                    </div>

                    {!isUnderDev &&
                      canInstall && (
                        <div className="flex items-center gap-1.5 flex-wrap">
                          <button
                            type="button"
                            disabled={isInstalling}
                            onClick={() => {
                              setLocalInstallOpenId(
                                isLocalPanelOpen ? null : comp.id,
                              );
                              setLocalSourceInput("");
                              setLocalSha256Input(
                                (mirrorEntry
                                  ? resolveHostMirrorPackage(
                                      mirrorEntry,
                                      hostPlatformTag,
                                      t,
                                    ).sha256
                                  : "") ||
                                  mirrorEntry?.sha256 ||
                                  "",
                              );
                            }}
                            data-testid={`open-local-install-btn-${comp.id}`}
                            className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer disabled:opacity-50"
                          >
                            {t("shell.pluginsPage.installFromLocal")}
                          </button>

                          {mirrorEntry && (() => {
                            const hostPkg = resolveHostMirrorPackage(
                              mirrorEntry,
                              hostPlatformTag,
                              t,
                            );
                            const mirrorDisabled =
                              isInstalling || !hostPkg.available;
                            return (
                              <button
                                type="button"
                                disabled={mirrorDisabled}
                                aria-busy={isInstalling}
                                title={hostPkg.disabledReason}
                                onClick={() =>
                                  void handleInstallFromMirror(
                                    comp.id,
                                    mirrorEntry,
                                  )
                                }
                                data-testid={`mirror-install-btn-${comp.id}`}
                                className="inline-flex h-7 min-w-[116px] justify-center items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-card px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
                              >
                                {isInstalling && showInstallingSpinner && (
                                  <Loader2 className="h-3 w-3 animate-spin shrink-0" strokeWidth={1.75} />
                                )}
                                <span>
                                  {hostPkg.available
                                    ? `${t("shell.pluginsPage.installFromMirror")} (v${mirrorEntry.version})`
                                    : hostPkg.disabledReason || t("shell.pluginsPage.packagePendingRelease")}
                                </span>
                              </button>
                            );
                          })()}
                        </div>
                      )}
                  </div>

                  <p
                    data-testid={`runtime-capability-${comp.id}`}
                    className="mt-1.5 text-xs text-sp-text-secondary leading-relaxed"
                  >
                    <span className="font-medium text-sp-text">
                      {comp.id === "eostage-core"
                        ? t("shell.pluginsPage.coreCapabilitiesPrefix")
                        : isUnderDev
                          ? t("shell.pluginsPage.plannedCapabilitiesPrefix")
                          : t("shell.pluginsPage.availableCapabilitiesPrefix")}
                    </span>
                    {capabilityLine}
                  </p>

                  {canInstall && isLocalPanelOpen && (
                    <form
                      onSubmit={(e) =>
                        handleInstallFromLocal(e, comp.id)
                      }
                      data-testid={`local-install-panel-${comp.id}`}
                      className="mt-3 rounded-sp-sm border border-sp-line bg-sp-card p-3 space-y-2.5"
                    >
                      <div className="flex items-center justify-between">
                        <span className="text-xs font-semibold text-sp-text">
                          {t("shell.pluginsPage.installFromLocal")} ({comp.name})
                        </span>
                        <button
                          type="button"
                          onClick={() => setLocalInstallOpenId(null)}
                          className="text-sp-text-muted hover:text-sp-text cursor-pointer"
                        >
                          <X className="h-3.5 w-3.5" strokeWidth={1.75} />
                        </button>
                      </div>
                      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                        <input
                          type="text"
                          value={localSourceInput}
                          onChange={(e) =>
                            setLocalSourceInput(e.target.value)
                          }
                          placeholder={t(
                            "shell.pluginsPage.localSourcePlaceholder",
                          )}
                          data-testid={`local-install-path-input-${comp.id}`}
                          className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                        />
                        <input
                          type="text"
                          value={localSha256Input}
                          onChange={(e) =>
                            setLocalSha256Input(e.target.value)
                          }
                          placeholder={t(
                            "shell.pluginsPage.expectedSha256Placeholder",
                          )}
                          data-testid={`local-install-sha256-input-${comp.id}`}
                          className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                        />
                      </div>
                      <div className="flex justify-end gap-2">
                        <button
                          type="button"
                          onClick={() => setLocalInstallOpenId(null)}
                          className="h-7 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-xs text-sp-text-secondary hover:text-sp-text whitespace-nowrap cursor-pointer"
                        >
                          {t("shell.pluginsPage.cancel")}
                        </button>
                        <button
                          type="submit"
                          disabled={isInstalling}
                          data-testid={`confirm-local-install-btn-${comp.id}`}
                          className="h-7 px-3 rounded-sp-sm border border-sp-line bg-sp-text text-xs font-medium text-sp-surface hover:opacity-90 whitespace-nowrap cursor-pointer disabled:opacity-50"
                        >
                          {isInstalling
                            ? t("shell.pluginsPage.installing")
                            : t("shell.pluginsPage.confirmInstall")}
                        </button>
                      </div>
                    </form>
                  )}

                  {(isReady || isPartial) && (
                    <div className="mt-3 grid grid-cols-2 sm:grid-cols-4 gap-2 rounded bg-sp-card p-2 text-xs border border-sp-line/50">
                      <div>
                        <span className="text-sp-text-muted block">{t("shell.pluginsPage.versionLabel")}</span>
                        <span className="font-semibold text-sp-text font-mono truncate block">
                          {comp.version}
                        </span>
                      </div>
                      <div>
                        <span className="text-sp-text-muted block">
                          {t("shell.pluginsPage.fileCount")}
                        </span>
                        <span className="font-semibold text-sp-text font-mono block">
                          {comp.file_count > 0
                            ? t("shell.pluginsPage.fileCountValue", { count: comp.file_count })
                            : "--"}
                        </span>
                      </div>
                      <div>
                        <span className="text-sp-text-muted block">
                          {t("shell.pluginsPage.size")}
                        </span>
                        <span className="font-semibold text-sp-text font-mono block">
                          {formatBytes(comp.total_bytes)}
                        </span>
                      </div>
                      <div>
                        <span className="text-sp-text-muted block">
                          {t("shell.pluginsPage.installPath")}
                        </span>
                        <span className="font-mono text-sp-text truncate block">
                          {comp.install_root}
                        </span>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
      </section>

      <section
        aria-label={t("shell.pluginsPage.externalGisTitle")}
        data-testid="external-gis-panel"
        className="rounded-sp-md border border-sp-line bg-sp-surface p-4 space-y-3"
      >
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h4 className="text-sm font-semibold text-sp-text">
              {t("shell.pluginsPage.externalGisTitle")}
            </h4>
            <p className="mt-0.5 text-xs text-sp-text-secondary">
              {t("shell.pluginsPage.externalGisSubtitle")}
            </p>
          </div>
          <button
            type="button"
            onClick={() => void fetchExternalGis(true)}
            disabled={externalGisLoading}
            data-testid="refresh-external-gis-btn"
            className="inline-flex h-8 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-card px-3 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer shrink-0 disabled:opacity-50"
          >
            <RefreshCw
              className={cn(
                "h-3.5 w-3.5",
                externalGisLoading && "animate-spin",
              )}
              strokeWidth={1.75}
            />
            <span>{t("shell.pluginsPage.externalGisRefresh")}</span>
          </button>
        </div>

        {externalGisBanner && (
          <div
            data-testid="external-gis-banner"
            className={cn(
              "flex items-center justify-between gap-2 rounded-sp-sm border px-3 py-2 text-xs",
              externalGisBanner.type === "success"
                ? "bg-sp-success-bg text-sp-success-text border-sp-line"
                : "bg-sp-danger-bg text-sp-danger-text border-sp-line",
            )}
          >
            <span>{externalGisBanner.text}</span>
            <button
              type="button"
              onClick={() => setExternalGisBanner(null)}
              className="text-current opacity-70 hover:opacity-100 cursor-pointer"
            >
              <X className="h-3.5 w-3.5" strokeWidth={1.75} />
            </button>
          </div>
        )}

        <div className="grid grid-cols-1 gap-2.5">
          {(externalGisReport?.toolchains || []).map((item) => {
            const connectorId =
              item.id === "rustgis"
                ? "plugin-mcp-rustgis"
                : `external-mcp-${item.id}`;
            const isMcpRegistered = customConnectors.some(
              (c) => c.id === connectorId && c.enabled,
            );
            const sourceLabel =
              item.source === "custom_path"
                ? t("shell.pluginsPage.externalGisSourceCustom")
                : item.source === "bundled_component"
                  ? t("shell.pluginsPage.externalGisSourceBundled")
                  : item.source === "unsupported_os"
                    ? t("shell.pluginsPage.externalGisUnsupportedOs")
                    : item.installed
                      ? t("shell.pluginsPage.externalGisSourceSystem")
                      : t("shell.pluginsPage.externalGisSourceMissing");
            const extCapability = externalGisCapabilitySummary(
              item.id,
              EXTERNAL_GIS_CAPABILITY_SUMMARY[item.id],
              t,
            );
            return (
              <div
                key={item.id}
                data-testid={`external-gis-card-${item.id}`}
                className="rounded-sp-sm border border-sp-line bg-sp-card p-3 space-y-2"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs font-semibold text-sp-text">
                      {item.display_name}
                    </span>
                    <span
                      className={cn(
                        "inline-flex items-center gap-1 text-xs font-medium whitespace-nowrap",
                        item.installed
                          ? "text-sp-success-text"
                          : "text-sp-text-muted",
                      )}
                    >
                      {item.installed ? (
                        <CheckCircle2 className="h-3 w-3" strokeWidth={1.75} />
                      ) : (
                        <XCircle className="h-3 w-3" strokeWidth={1.75} />
                      )}
                      <span>{sourceLabel}</span>
                    </span>
                    {item.version && (
                      <span className="font-mono text-xs text-sp-text-secondary">
                        {item.version}
                      </span>
                    )}
                  </div>

                  {item.mcp_capable && item.installed && (
                    <button
                      type="button"
                      data-testid={`register-${item.id}-mcp-btn`}
                      onClick={() => handleRegisterExternalGisMcp(item)}
                      className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer"
                    >
                      {isMcpRegistered ? (
                        <Check className="h-3 w-3 text-sp-success-text" strokeWidth={1.75} />
                      ) : (
                        <Cable className="h-3 w-3" strokeWidth={1.75} />
                      )}
                      <span>
                        {isMcpRegistered
                          ? t("shell.pluginsPage.externalGisMcpRegistered")
                          : t("shell.pluginsPage.externalGisRegisterMcp")}
                      </span>
                    </button>
                  )}
                </div>

                {extCapability && (
                  <p className="text-xs text-sp-text-secondary">
                    <span className="font-medium text-sp-text">{t("shell.pluginsPage.connectedCapabilitiesPrefix")}</span>
                    {extCapability}
                  </p>
                )}

                {item.installed && item.executable_path && (
                  <p className="font-mono text-xs text-sp-text-muted break-all">
                    {item.executable_path}
                  </p>
                )}

                {item.source !== "unsupported_os" && (
                  <div className="flex items-center gap-2">
                    <input
                      type="text"
                      value={externalGisPaths[item.id] ?? ""}
                      onChange={(e) =>
                        setExternalGisPaths((prev) => ({
                          ...prev,
                          [item.id]: e.target.value,
                        }))
                      }
                      placeholder={t(
                        "shell.pluginsPage.externalGisCustomPathPlaceholder",
                      )}
                      data-testid={`external-gis-path-input-${item.id}`}
                      className="h-7 flex-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                    />
                    <button
                      type="button"
                      onClick={() => void handleSaveExternalGisPath(item.id)}
                      data-testid={`external-gis-save-path-${item.id}`}
                      className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer shrink-0"
                    >
                      {t("shell.pluginsPage.externalGisSavePath")}
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>

        {/* 日志压缩与可选外部 RTK 路径 */}
        {(() => {
          const dehydrator = externalGisReport?.log_dehydrator;
          const activeMode =
            externalGisReport?.prefs?.dehydrator_mode ||
            dehydrator?.mode ||
            "auto";
          const sharedRtk = dehydrator?.shared_rtk;
          return (
            <section
              aria-label={t("shell.pluginsPage.logDehydratorTitle")}
              data-testid="log-dehydrator-card"
              className="rounded-sp-sm border border-sp-line bg-sp-card p-3 space-y-2"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-xs font-semibold text-sp-text">
                    {t("shell.pluginsPage.logDehydratorTitle")}
                  </span>
                  <span className="inline-flex items-center gap-1 text-xs font-medium text-sp-success-text whitespace-nowrap">
                    <CheckCircle2 className="h-3 w-3" strokeWidth={1.75} />
                    <span>{t("shell.pluginsPage.logDehydratorReady")}</span>
                  </span>
                  {sharedRtk?.installed && (
                    <span className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-success-bg px-2 py-0.5 text-xs text-sp-success-text whitespace-nowrap">
                      <span>{t("shell.pluginsPage.sharedRtkReady")}</span>
                      {sharedRtk?.version && (
                        <span className="font-mono">{sharedRtk.version}</span>
                      )}
                    </span>
                  )}
                </div>

                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    data-testid="dehydrator-mode-auto"
                    onClick={() => void handleSetDehydratorMode("auto")}
                    className={cn(
                      "inline-flex h-7 items-center rounded-sp-sm border px-2.5 text-xs font-medium whitespace-nowrap cursor-pointer transition-colors",
                      activeMode !== "builtin_only"
                        ? "border-sp-accent bg-sp-surface text-sp-text"
                        : "border-sp-line bg-sp-surface text-sp-text-muted hover:bg-sp-hover",
                    )}
                  >
                    {t("shell.pluginsPage.logDehydratorModeAuto")}
                  </button>
                  <button
                    type="button"
                    data-testid="dehydrator-mode-builtin"
                    onClick={() => void handleSetDehydratorMode("builtin_only")}
                    className={cn(
                      "inline-flex h-7 items-center rounded-sp-sm border px-2.5 text-xs font-medium whitespace-nowrap cursor-pointer transition-colors",
                      activeMode === "builtin_only"
                        ? "border-sp-accent bg-sp-surface text-sp-text"
                        : "border-sp-line bg-sp-surface text-sp-text-muted hover:bg-sp-hover",
                    )}
                  >
                    {t("shell.pluginsPage.logDehydratorModeBuiltin")}
                  </button>
                </div>
              </div>

              <p className="text-xs text-sp-text-secondary">
                {t("shell.pluginsPage.logDehydratorBody")}
              </p>

              {sharedRtk?.executable_path && (
                <p className="font-mono text-xs text-sp-text-muted break-all">
                  {sharedRtk.executable_path}
                </p>
              )}

              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={rtkCustomPath}
                  onChange={(e) => setRtkCustomPath(e.target.value)}
                  placeholder={t("shell.pluginsPage.sharedRtkCustomPlaceholder")}
                  data-testid="shared-rtk-path-input"
                  className="h-7 flex-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />
                <button
                  type="button"
                  onClick={() => void handleSaveRtkPath()}
                  data-testid="shared-rtk-save-btn"
                  className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer shrink-0"
                >
                  {t("shell.pluginsPage.sharedRtkSavePath")}
                </button>
              </div>
            </section>
          );
        })()}

        {/* 本机 Conda / Python 科学计算环境发现 */}
        {(() => {
          const condaEnvs = externalGisReport?.conda_environments || [];
          const preferredConda = externalGisReport?.prefs?.preferred_conda_env;
          return (
            <section
              aria-label={t("shell.pluginsPage.condaEnvsTitle")}
              data-testid="external-conda-section"
              className="rounded-sp-sm border border-sp-line bg-sp-card p-3 space-y-2.5"
            >
              <div>
                <h5 className="text-xs font-semibold text-sp-text">
                  {t("shell.pluginsPage.condaEnvsTitle")}
                </h5>
                <p className="mt-0.5 text-xs text-sp-text-secondary">
                  {t("shell.pluginsPage.condaEnvsSubtitle")}
                </p>
              </div>

              {condaEnvs.length === 0 ? (
                <p className="text-xs text-sp-text-muted">
                  {t("shell.pluginsPage.condaEnvsEmpty")}
                </p>
              ) : (
                <div className="grid grid-cols-1 gap-2">
                  {condaEnvs.map((env) => {
                    const isDefault = preferredConda === env.name;
                    const isCustom = env.source === "custom";
                    return (
                      <div
                        key={env.id}
                        data-testid={`conda-env-card-${env.name}`}
                        className="rounded-sp-sm border border-sp-line bg-sp-surface p-2.5 space-y-1.5"
                      >
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div className="flex items-center gap-2 flex-wrap">
                            <span className="text-xs font-semibold text-sp-text">
                              {env.name}
                            </span>
                            <span className="rounded bg-sp-card px-1.5 py-0.5 text-xs text-sp-text-secondary border border-sp-line">
                              {isCustom
                                ? t("shell.pluginsPage.condaSourceCustom")
                                : t("shell.pluginsPage.condaSourceAuto")}
                            </span>
                            {env.python_version && (
                              <span className="font-mono text-xs text-sp-text-muted">
                                {env.python_version}
                              </span>
                            )}
                          </div>

                          {isDefault ? (
                            <span
                              data-testid={`conda-default-badge-${env.name}`}
                              className="inline-flex h-6 items-center gap-1 rounded-full bg-sp-success-bg text-sp-success-text px-2 text-xs font-medium whitespace-nowrap"
                            >
                              <Check className="h-3 w-3" strokeWidth={1.75} />
                              <span>{t("shell.pluginsPage.condaDefaultBadge")}</span>
                            </span>
                          ) : (
                            <button
                              type="button"
                              data-testid={`conda-set-default-${env.name}`}
                              onClick={() => void handleSetPreferredCondaEnv(env.name)}
                              className="inline-flex h-6 items-center rounded-sp-sm border border-sp-line bg-sp-card px-2 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer"
                            >
                              {t("shell.pluginsPage.condaSetDefault")}
                            </button>
                          )}
                        </div>

                        {env.geo_packages.length > 0 && (
                          <div className="flex flex-wrap gap-1">
                            {env.geo_packages.map((pkg) => (
                              <code
                                key={pkg}
                                className="rounded bg-sp-card border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text-secondary"
                              >
                                {pkg}
                              </code>
                            ))}
                          </div>
                        )}

                        <p className="font-mono text-xs text-sp-text-muted break-all">
                          {env.python_path}
                        </p>
                      </div>
                    );
                  })}
                </div>
              )}

              <div className="flex flex-wrap items-center gap-2 pt-1">
                <input
                  type="text"
                  value={condaAliasInput}
                  onChange={(e) => setCondaAliasInput(e.target.value)}
                  placeholder={t("shell.pluginsPage.condaCustomAliasPlaceholder")}
                  data-testid="conda-custom-alias-input"
                  className="h-7 w-36 rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />
                <input
                  type="text"
                  value={condaPathInput}
                  onChange={(e) => setCondaPathInput(e.target.value)}
                  placeholder={t("shell.pluginsPage.condaCustomPathPlaceholder")}
                  data-testid="conda-custom-path-input"
                  className="h-7 flex-1 min-w-48 rounded-sp-sm border border-sp-line bg-sp-surface px-2 font-mono text-xs text-sp-text focus:border-sp-text-secondary focus:outline-none"
                />
                <button
                  type="button"
                  onClick={() => void handleAddCustomCondaEnv()}
                  data-testid="conda-add-custom-btn"
                  disabled={!condaAliasInput.trim() || !condaPathInput.trim()}
                  className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover whitespace-nowrap cursor-pointer shrink-0 disabled:opacity-50"
                >
                  {t("shell.pluginsPage.condaAddCustomEnv")}
                </button>
              </div>

              {(() => {
                const detectedPackagesLower = new Set(
                  condaEnvs.flatMap((e) =>
                    (e.geo_packages || []).map((p) => p.toLowerCase()),
                  ),
                );
                return (
                  <div
                    data-testid="curated-conda-geo-packages"
                    className="pt-2.5 mt-2 border-t border-sp-line/70 space-y-2"
                  >
                    <div className="flex items-center justify-between gap-2 flex-wrap">
                      <span className="text-xs font-semibold text-sp-text">
                        {t("shell.pluginsPage.condaCuratedTitle")}
                      </span>
                      <span className="text-xs text-sp-text-muted">
                        {t("shell.pluginsPage.condaTargetEnv", {
                          env: preferredConda || condaEnvs[0]?.name || t("shell.pluginsPage.condaNoDefaultEnv"),
                        })}
                      </span>
                    </div>
                    <div className="grid grid-cols-1 gap-1.5">
                      {CURATED_CONDA_GEO_PACKAGES.map((pkg) => {
                        const detected = detectedPackagesLower.has(
                          pkg.id.toLowerCase(),
                        );
                        const busy = installingCondaPkgId === pkg.id;
                        return (
                          <div
                            key={pkg.id}
                            data-testid={`curated-conda-pkg-${pkg.id}`}
                            className="flex items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-2"
                          >
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center gap-1.5 flex-wrap">
                                <span className="text-xs font-semibold text-sp-text">
                                  {pkg.label}
                                </span>
                                {detected && (
                                  <span
                                    data-testid={`curated-conda-pkg-detected-${pkg.id}`}
                                    className="rounded-full bg-sp-success-bg px-1.5 py-0.5 text-xs font-medium text-sp-success-text"
                                  >
                                    {t("shell.pluginsPage.condaDetectedBadge")}
                                  </span>
                                )}
                                <code className="font-mono text-xs text-sp-text-muted">
                                  {pkg.commandPreview}
                                </code>
                              </div>
                              <p className="mt-0.5 text-xs text-sp-text-secondary">
                                {condaPackageSummary(pkg.id, pkg.summary, t)}
                              </p>
                            </div>
                            <button
                              type="button"
                              data-testid={`install-conda-pkg-btn-${pkg.id}`}
                              disabled={Boolean(installingCondaPkgId)}
                              onClick={(e) =>
                                setPendingCondaInstall({
                                  pkgId: pkg.id,
                                  label: pkg.label,
                                  commandPreview: pkg.commandPreview,
                                  targetEnv:
                                    preferredConda || condaEnvs[0]?.name || null,
                                  anchorEl: e.currentTarget,
                                })
                              }
                              className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-card px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer shrink-0 disabled:opacity-50"
                            >
                              {busy && (
                                <Loader2 className="h-3 w-3 animate-spin" strokeWidth={1.75} />
                              )}
                              <span>
                                {busy
                                  ? t("shell.pluginsPage.condaInstallingBtn")
                                  : detected
                                    ? t("shell.pluginsPage.condaReinstallBtn")
                                    : t("shell.pluginsPage.condaInstallBtn")}
                              </span>
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })()}
            </section>
          );
        })()}
      </section>

      <Popconfirm
        open={pendingCondaInstall !== null}
        anchorEl={pendingCondaInstall?.anchorEl}
        title={
          pendingCondaInstall
            ? t("shell.pluginsPage.condaConfirmTitle", { label: pendingCondaInstall.label })
            : ""
        }
        description={
          pendingCondaInstall
            ? t("shell.pluginsPage.condaConfirmDesc", {
                env: pendingCondaInstall.targetEnv || t("shell.pluginsPage.condaDefaultFallbackEnv"),
                cmd: pendingCondaInstall.commandPreview,
              })
            : undefined
        }
        confirmLabel={t("shell.pluginsPage.condaConfirmBtn")}
        cancelLabel={t("shell.pluginsPage.cancel")}
        tone="primary"
        onCancel={() => setPendingCondaInstall(null)}
        onConfirm={() => {
          const cur = pendingCondaInstall;
          setPendingCondaInstall(null);
          if (!cur) return;
          void (async () => {
            setInstallingCondaPkgId(cur.pkgId);
            setExternalGisBanner(null);
            const lwId = startLongWork({
              title: t("shell.pluginsPage.condaInstallLongTitle", { label: cur.label }),
              category: "plugin",
              stageLabel: t("shell.pluginsPage.condaInstallLongStage", {
                label: cur.label,
                env: cur.targetEnv || t("shell.pluginsPage.condaDefaultFallbackEnv"),
              }),
            });
            try {
              const res = await installCondaGeoPackageBackend({
                package_id: cur.pkgId,
                conda_env: cur.targetEnv,
                confirmed: true,
              });
              if (!res) {
                throw new Error(t("shell.pluginsPage.condaHostOnlyErr"));
              }
              if (!res.ok) {
                throw new Error(res.detail || res.stderr || t("shell.pluginsPage.condaInstallFailed", { label: cur.label }));
              }
              await fetchExternalGis(false);
              setExternalGisBanner({
                type: "success",
                text: res.detail || t("shell.pluginsPage.condaInstalledBanner", { label: cur.label, env: res.env_name }),
              });
              emitTopToast({ text: t("shell.pluginsPage.condaInstalledToast", { label: cur.label }), tone: "success" });
              completeLongWork(lwId, {
                doneTitle: t("shell.pluginsPage.condaReadyTitle", { label: cur.label }),
                doneSubtitle: t("shell.pluginsPage.condaReadySubtitle", { env: res.env_name }),
              });
            } catch (err: any) {
              const msg =
                err?.context ||
                err?.message ||
                String(err || t("shell.pluginsPage.condaInstallFailed", { label: cur.label }));
              setExternalGisBanner({ type: "error", text: msg });
              failLongWork(lwId, msg);
            } finally {
              setInstallingCondaPkgId(null);
            }
          })();
        }}
      />
    </div>
  );
}
