import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Download,
  Globe,
  Layers,
  Loader2,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import type { Bbox } from "@/lib/bridge";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { SpSelect } from "@/components/ui/SpSelect";
import { useDelayedFlag, useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { formatBytes } from "@/shell/api/agentStreamLogic";
import { CATALOG_METADATA_REGISTRY } from "@/lib/dataSourceCatalogLogic";
import { GEE_PUBLIC_CATALOG_INDEX, isStaticGeeAssetId } from "@/lib/geePublicCatalogIndex";
import {
  GEE_QUOTA_CONFIGURATION_URL,
  GEE_SERVICE_ACCOUNT_CONSOLE_URL,
  formatLocalizedGeeTierLabel,
  geeArtifactGet,
  geeCatalogSearch,
  geeCheckAvailability,
  geeChunkedDownload,
  geeCloudAnalyze,
  geeQuotaStatus,
  pushGeeArtifactToMap,
  type GeeArtifactRecord,
  type GeeAvailabilityResult,
  type GeeCatalogItem,
  type GeeQuotaStatusResponse,
  type GeeSceneSummary,
} from "@/lib/geeCloudSuiteClient";
import {
  getWorkbenchToolOptions,
  subscribeWorkbenchToolOptions,
  updateWorkbenchToolOptions,
} from "./ManualConditionsPanel";

export { GEE_QUOTA_CONFIGURATION_URL, GEE_SERVICE_ACCOUNT_CONSOLE_URL };

type ExtendedGeeAvailability = GeeAvailabilityResult & {
  start_date?: string;
  end_date?: string;
  bands?: string[];
};

export type GeeDatasetKind =
  | "optical"
  | "sar"
  | "dem"
  | "climate"
  | "lulc"
  | "generic";

export interface GeeAnalysisOptionItem {
  value: string;
  label: string;
  operation: "composite" | "index" | "change_detect";
  indexName?: string;
}

/**
 * 统一解包 Tauri / GEE 结构化错误对象，彻底杜绝 "[object Object]" 显示。
 */
export function extractGeeErrorMessage(err: unknown, depth = 0): string {
  if (depth > 4) {
    return "GEE 服务调用失败，请稍后重试。";
  }
  if (err == null) {
    return "GEE 服务调用失败（未返回错误详情）。";
  }
  if (typeof err === "string") {
    const trimmed = err.trim();
    if (!trimmed || trimmed === "[object Object]") {
      return "GEE 服务返回了非文本错误对象，请检查研究区范围与账号权限后重试。";
    }
    if (trimmed.startsWith("{") && trimmed.endsWith("}")) {
      try {
        const parsed = JSON.parse(trimmed);
        return extractGeeErrorMessage(parsed, depth + 1);
      } catch {
        // Keep raw string when not valid JSON
      }
    }
    return trimmed;
  }
  if (err instanceof Error) {
    const msg = (err.message || "").trim();
    if (msg && msg !== "[object Object]") {
      if (msg.startsWith("{") && msg.endsWith("}")) {
        try {
          return extractGeeErrorMessage(JSON.parse(msg), depth + 1);
        } catch {
          return msg;
        }
      }
      return msg;
    }
    const cause = (err as Error & { cause?: unknown }).cause;
    if (cause !== undefined) {
      return extractGeeErrorMessage(cause, depth + 1);
    }
  }
  if (typeof err === "object") {
    const obj = err as Record<string, unknown>;
    const candidateKeys = [
      "message_zh",
      "context",
      "message",
      "error",
      "reason",
      "detail",
      "details",
      "cause",
    ];
    for (const key of candidateKeys) {
      const val = obj[key];
      if (val !== undefined && val !== null) {
        const extracted = extractGeeErrorMessage(val, depth + 1);
        if (extracted && extracted !== "[object Object]") {
          const codeStr =
            typeof obj.code === "string" && obj.code.trim() ? obj.code.trim() : "";
          if (codeStr && !extracted.includes(codeStr)) {
            return `[${codeStr}] ${extracted}`;
          }
          return extracted;
        }
      }
    }
    if (typeof obj.code === "string" && obj.code.trim()) {
      return `GEE 错误 (${obj.code.trim()})`;
    }
    try {
      const serialized = JSON.stringify(obj);
      if (serialized && serialized !== "{}") {
        return serialized;
      }
    } catch {
      // Ignore circular reference
    }
  }
  const coerced = String(err);
  return coerced && coerced !== "[object Object]"
    ? coerced
    : "GEE 服务调用失败，请稍后重试。";
}

function isTooLargeGridError(err: unknown, messageText: string): boolean {
  const obj = err && typeof err === "object" ? (err as Record<string, unknown>) : null;
  const code = obj && typeof obj.code === "string" ? obj.code : "";
  const ctx = obj && typeof obj.context === "string" ? obj.context : "";
  const combined = [messageText, ctx, code].filter(Boolean).join("\n");
  if (
    code === "GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD" ||
    combined.includes("GEE_AOI_TOO_LARGE_FOR_SINGLE_DOWNLOAD")
  ) {
    return true;
  }
  return /网格[分]?块数?.*64|chunks?.*64|超过\s*64|>\s*64\s*(?:个?分?块|chunks?)|single download/i.test(
    combined
  );
}

function shouldSkipGeeTemporalFilter(
  assetId: string,
  datasetKind: GeeDatasetKind,
  catalogItem?: GeeCatalogItem | null
): boolean {
  if (datasetKind === "dem" || catalogItem?.asset_type === "image") {
    return true;
  }
  return isStaticGeeAssetId(assetId);
}

function normalizeCatalogTags(tags: unknown): string[] {
  if (Array.isArray(tags)) {
    return tags.map((v) => String(v ?? "").trim()).filter(Boolean);
  }
  if (typeof tags === "string") {
    return tags
      .split(/[\s,;，；]+/)
      .map((v) => v.trim())
      .filter(Boolean);
  }
  return [];
}

/**
 * 判定当前 GEE Asset 所属数据集家族，用于自适应过滤云端算子并隐藏无意义的光学云量控件。
 */
export function classifyGeeDatasetKind(
  assetId: string,
  catalogItem?: GeeCatalogItem | null
): GeeDatasetKind {
  const normalized = assetId.trim().toUpperCase();
  const publicMatch = GEE_PUBLIC_CATALOG_INDEX.find(
    (entry) => entry.assetId.toUpperCase() === normalized
  );
  if (publicMatch) {
    if (publicMatch.theme === "sar") return "sar";
    if (publicMatch.theme === "dem") return "dem";
    if (publicMatch.theme === "climate" || publicMatch.theme === "atmosphere") {
      return "climate";
    }
    if (
      publicMatch.theme === "landcover" ||
      publicMatch.theme === "water" ||
      publicMatch.theme === "ecology"
    ) {
      return "lulc";
    }
    if (publicMatch.theme === "optical") return "optical";
  }

  const tagBag = normalizeCatalogTags(catalogItem?.tags).join(" ").toUpperCase();
  if (
    normalized.includes("S1_GRD") ||
    normalized.includes("PALSAR") ||
    normalized.includes("NISAR") ||
    normalized.includes("SAR") ||
    tagBag.includes("SAR")
  ) {
    return "sar";
  }
  if (
    normalized.includes("/DEM/") ||
    normalized.includes("NASADEM") ||
    normalized.includes("SRTM") ||
    normalized.includes("AW3D30") ||
    normalized.includes("FABDEM") ||
    normalized.includes("MERIT") ||
    tagBag.includes("DEM")
  ) {
    return "dem";
  }
  if (
    normalized.includes("ERA5") ||
    normalized.includes("MODIS/") ||
    normalized.includes("MOD13") ||
    normalized.includes("GPM") ||
    normalized.includes("CHIRPS") ||
    normalized.includes("GLDAS") ||
    normalized.includes("FLDAS") ||
    normalized.includes("SMAP") ||
    normalized.includes("S5P") ||
    normalized.includes("TROPOMI") ||
    normalized.includes("DAYMET")
  ) {
    return "climate";
  }
  if (
    normalized.includes("DYNAMIC_WORLD") ||
    normalized.includes("WORLDCOVER") ||
    normalized.includes("CLCD") ||
    normalized.includes("LANDCOVER") ||
    normalized.includes("LAND_COVER") ||
    normalized.includes("GLC") ||
    normalized.includes("LULC") ||
    normalized.includes("GSW") ||
    normalized.includes("MCD12") ||
    normalized.includes("GHSL") ||
    normalized.includes("VIIRS") ||
    tagBag.includes("LAND COVER") ||
    tagBag.includes("LANDCOVER") ||
    tagBag.includes("LULC") ||
    tagBag.includes("土地覆盖") ||
    tagBag.includes("土地利用")
  ) {
    return "lulc";
  }
  if (
    normalized.includes("COPERNICUS/S2") ||
    normalized.includes("LANDSAT") ||
    normalized.includes("HLS") ||
    normalized.includes("MOD09")
  ) {
    return "optical";
  }
  return "generic";
}

function datasetKindBadgeLabel(kind: GeeDatasetKind, locale: string): string {
  switch (kind) {
    case "sar":
      return locale === "en" ? "Radar SAR" : "雷达 SAR";
    case "dem":
      return locale === "en" ? "DEM / Terrain" : "高程 DEM";
    case "climate":
      return locale === "en" ? "Climate / Met" : "气象与环境";
    case "lulc":
      return locale === "en" ? "LULC / Thematic" : "地表专题";
    case "optical":
      return locale === "en" ? "Optical" : "光学影像";
    default:
      return locale === "en" ? "EE Dataset" : "通用数据集";
  }
}

function findMatchedGeePresetId(assetId: string): string {
  const normalized = assetId.trim();
  const publicMatch = GEE_PUBLIC_CATALOG_INDEX.find((e) => e.assetId === normalized);
  if (publicMatch?.presetResourceId) {
    return publicMatch.presetResourceId;
  }
  const metaMatch = Object.entries(CATALOG_METADATA_REGISTRY).find(
    ([resId, meta]) =>
      resId !== "science.gee.catalog.v1" &&
      resId.startsWith("science.gee.") &&
      meta.geeAssetId === normalized
  );
  return metaMatch ? metaMatch[0] : "science.gee.catalog.v1";
}

function formatQuotaHours(hours: number): string {
  if (!Number.isFinite(hours)) return "0";
  if (Number.isInteger(hours)) return String(hours);
  const oneDec = Math.round(hours * 10) / 10;
  if (Math.abs(hours - oneDec) < 1e-4) {
    return Number.isInteger(oneDec) ? String(oneDec) : oneDec.toFixed(1);
  }
  return Number(hours.toFixed(2)).toString();
}

function isValidQuotaStatus(val: unknown): val is GeeQuotaStatusResponse {
  return Boolean(
    val &&
      typeof val === "object" &&
      typeof (val as GeeQuotaStatusResponse).account_tier === "string" &&
      (val as GeeQuotaStatusResponse).cloud_monitoring &&
      (val as GeeQuotaStatusResponse).local_ledger
  );
}

export interface GeeSyncedResultsPayload {
  assetId: string;
  title: string;
  bbox: Bbox;
  startDate: string | null;
  endDate: string | null;
  maxCloud: number;
  nativeScaleM: number;
  estimatedMb?: number;
  totalScenes: number;
  matchedScenes: number;
  scenes: GeeSceneSummary[];
  latestArtifact?: GeeArtifactRecord | null;
}

export interface GeeWorkbenchPanelProps {
  workingAoi: Bbox | null;
  aoiLabel: string;
  startDate: string | null;
  endDate: string | null;
  onStartDateChange: (val: string) => void;
  onEndDateChange: (val: string) => void;
  onOpenCatalog?: () => void;
  onSyncGeeResults?: (payload: GeeSyncedResultsPayload) => void;
}

export function GeeWorkbenchPanel({
  workingAoi,
  aoiLabel,
  startDate,
  endDate,
  onStartDateChange,
  onEndDateChange,
  onOpenCatalog,
  onSyncGeeResults,
}: GeeWorkbenchPanelProps) {
  const { locale, t } = useI18n();
  const [searchQuery, setSearchQuery] = useState("");
  const [catalogItems, setCatalogItems] = useState<GeeCatalogItem[]>([]);
  const [selectedAssetId, setSelectedAssetId] = useState<string>(() => {
    const initial = getWorkbenchToolOptions().geeAssetId;
    return initial && initial !== "ALL" ? initial : "COPERNICUS/S2_SR_HARMONIZED";
  });
  const [selectedCatalogItem, setSelectedCatalogItem] = useState<GeeCatalogItem | null>(null);

  // Sync bidirectionally with DataSourceCatalogModal / ManualConditionsPanel
  useEffect(() => {
    const applySnapshot = () => {
      const opts = getWorkbenchToolOptions();
      const nextAsset =
        opts.geeAssetId && opts.geeAssetId !== "ALL"
          ? opts.geeAssetId
          : null;
      if (nextAsset) {
        setSelectedAssetId((prev) => {
          if (prev === nextAsset) return prev;
          setAvailability(null);
          setSelectedCatalogItem((cur) => (cur && cur.id === nextAsset ? cur : null));
          return nextAsset;
        });
      }
    };
    applySnapshot();
    return subscribeWorkbenchToolOptions(applySnapshot);
  }, []);

  const selectAssetUnified = useCallback(
    (nextAssetId: string, item?: GeeCatalogItem | null) => {
      const normalized = nextAssetId.trim();
      setSelectedAssetId(nextAssetId);
      if (item !== undefined) {
        setSelectedCatalogItem(item);
      } else {
        setSelectedCatalogItem((prev) => (prev && prev.id === normalized ? prev : null));
      }
      setAvailability(null);
      setTooLargeError(null);
      if (normalized && normalized !== "ALL") {
        const curOpts = getWorkbenchToolOptions();
        const presetId = findMatchedGeePresetId(normalized);
        if (
          curOpts.geeAssetId !== normalized ||
          curOpts.geeSelectedPresetId !== presetId
        ) {
          updateWorkbenchToolOptions({
            geeAssetId: normalized,
            geeSelectedPresetId: presetId,
          });
        }
      }
    },
    []
  );

  const matchedPublicEntry = useMemo(
    () =>
      GEE_PUBLIC_CATALOG_INDEX.find(
        (entry) => entry.assetId.toUpperCase() === selectedAssetId.trim().toUpperCase()
      ) || null,
    [selectedAssetId]
  );

  const selectedTitle = selectedCatalogItem
    ? locale === "zh-CN"
      ? selectedCatalogItem.title_zh || selectedCatalogItem.title_en || selectedCatalogItem.id
      : selectedCatalogItem.title_en || selectedCatalogItem.title_zh || selectedCatalogItem.id
    : selectedAssetId === "COPERNICUS/S2_SR_HARMONIZED"
    ? t("mapWorkbench.gee.defaultDatasetTitle")
    : matchedPublicEntry
    ? matchedPublicEntry.title
    : selectedAssetId;

  const datasetKind = useMemo(
    () => classifyGeeDatasetKind(selectedAssetId, selectedCatalogItem),
    [selectedAssetId, selectedCatalogItem]
  );

  const [analysisKey, setAnalysisKey] = useState("index_NDVI");
  const [maxCloud, setMaxCloud] = useState(25);
  const [afterStartDate, setAfterStartDate] = useState("2025-07-01");
  const [afterEndDate, setAfterEndDate] = useState("2025-10-01");
  const [thresholdText, setThresholdText] = useState("");

  const [availability, setAvailability] = useState<ExtendedGeeAvailability | null>(null);
  const [quotaPrompt, setQuotaPrompt] = useState<Record<string, any> | null>(null);
  const [tooLargeError, setTooLargeError] = useState<{
    message: string;
    suggestedScaleM?: number;
    totalChunks?: number;
  } | null>(null);
  const [busyMode, setBusyMode] = useState<
    "preflight" | "add_layer" | "analyze" | "download" | null
  >(null);
  const [elapsedMs, setElapsedMs] = useState(0);
  const geePanelRef = useRef<HTMLDivElement | null>(null);
  const requestGenRef = useRef(0);
  const [statusNote, setStatusNote] = useState<{ ok: boolean; text: string } | null>(null);
  const [workbenchQuota, setWorkbenchQuota] = useState<GeeQuotaStatusResponse | null>(null);
  const [refreshingWorkbenchQuota, setRefreshingWorkbenchQuota] = useState(false);
  const { spinClass: quotaRefreshSpinClass } = useFullTurnSpin(refreshingWorkbenchQuota);
  const showGeeActionSpinner = useDelayedFlag(busyMode !== null);

  useEffect(() => {
    const scrollParent = geePanelRef.current?.closest("[data-conditions-scroll]") as HTMLElement | null;
    if (scrollParent) {
      scrollParent.scrollTop = 0;
    }
  }, [selectedAssetId]);

  const aoiResetKey = workingAoi
    ? `${workingAoi.west},${workingAoi.south},${workingAoi.east},${workingAoi.north}`
    : "";
  useEffect(() => {
    setAvailability(null);
    setTooLargeError(null);
  }, [selectedAssetId, startDate, endDate, maxCloud, aoiResetKey]);

  const availableSceneCount = Number(
    availability?.cloud_filtered_scenes ?? availability?.total_scenes ?? 0
  );
  const preflightPassed = availability !== null && availableSceneCount > 0;

  // Adaptive operator choices tailored to the active dataset kind
  const analysisOptions = useMemo<GeeAnalysisOptionItem[]>(() => {
    const compositeOpt: GeeAnalysisOptionItem = {
      value: "composite_median",
      label: t("mapWorkbench.gee.opCompositeMedian"),
      operation: "composite",
    };

    if (datasetKind === "dem") {
      return [
        {
          value: "composite_median",
          label:
            locale === "en"
              ? "Elevation Render / Mosaic (Original Bands)"
              : "高程基准成图（原始栅格/地形渲染）",
          operation: "composite",
        },
      ];
    }

    if (datasetKind === "sar") {
      return [
        {
          value: "composite_median",
          label:
            locale === "en"
              ? "SAR Backscatter Median Composite (VV / VH)"
              : "SAR 后向散射中值合成 (VV / VH)",
          operation: "composite",
        },
        {
          value: "index_SAR_DIFF",
          label: t("mapWorkbench.gee.opIndexSarDiff"),
          operation: "index",
          indexName: "SAR_DIFF",
        },
        {
          value: "change_SAR",
          label:
            locale === "en"
              ? "Dual-Phase SAR Change Detection (Swipe)"
              : "双时相雷达后向散射变化检测（卷帘对比）",
          operation: "change_detect",
          indexName: "SAR_DIFF",
        },
      ];
    }

    if (datasetKind === "climate" || datasetKind === "lulc") {
      return [
        {
          value: "composite_median",
          label:
            locale === "en"
              ? "Thematic Median / Mean Map Layer"
              : "专题波段合成上屏（中值/均值渲染）",
          operation: "composite",
        },
        {
          value: "change_NDVI",
          label:
            locale === "en"
              ? "Dual-Period Change Swipe"
              : "双时相专题变化对比（卷帘对比）",
          operation: "change_detect",
          indexName: "NDVI",
        },
      ];
    }

    if (datasetKind === "generic") {
      return [
        compositeOpt,
        {
          value: "index_NDVI",
          label: t("mapWorkbench.gee.opIndexNdvi"),
          operation: "index",
          indexName: "NDVI",
        },
        {
          value: "index_NDWI",
          label: t("mapWorkbench.gee.opIndexNdwi"),
          operation: "index",
          indexName: "NDWI",
        },
        {
          value: "index_SAR_DIFF",
          label: t("mapWorkbench.gee.opIndexSarDiff"),
          operation: "index",
          indexName: "SAR_DIFF",
        },
        {
          value: "change_NDVI",
          label: t("mapWorkbench.gee.opChangeNdvi"),
          operation: "change_detect",
          indexName: "NDVI",
        },
      ];
    }

    // Optical datasets (Sentinel-2 / Landsat / HLS / etc.)
    return [
      compositeOpt,
      {
        value: "index_NDVI",
        label: t("mapWorkbench.gee.opIndexNdvi"),
        operation: "index",
        indexName: "NDVI",
      },
      {
        value: "index_NDWI",
        label: t("mapWorkbench.gee.opIndexNdwi"),
        operation: "index",
        indexName: "NDWI",
      },
      {
        value: "index_MNDWI",
        label: t("mapWorkbench.gee.opIndexMndwi"),
        operation: "index",
        indexName: "MNDWI",
      },
      {
        value: "index_NDBI",
        label: t("mapWorkbench.gee.opIndexNdbi"),
        operation: "index",
        indexName: "NDBI",
      },
      {
        value: "index_EVI",
        label: t("mapWorkbench.gee.opIndexEvi"),
        operation: "index",
        indexName: "EVI",
      },
      {
        value: "index_NBR",
        label: t("mapWorkbench.gee.opIndexNbr"),
        operation: "index",
        indexName: "NBR",
      },
      {
        value: "index_BSI",
        label: t("mapWorkbench.gee.opIndexBsi"),
        operation: "index",
        indexName: "BSI",
      },
      {
        value: "change_NDVI",
        label: t("mapWorkbench.gee.opChangeNdvi"),
        operation: "change_detect",
        indexName: "NDVI",
      },
    ];
  }, [datasetKind, locale, t]);

  // Keep selected operator valid and relevant when dataset kind switches
  useEffect(() => {
    const stillValid = analysisOptions.some((opt) => opt.value === analysisKey);
    if (!stillValid) {
      const preferred =
        datasetKind === "optical" && analysisOptions.some((o) => o.value === "index_NDVI")
          ? "index_NDVI"
          : analysisOptions[0]?.value || "composite_median";
      setAnalysisKey(preferred);
    }
  }, [analysisKey, analysisOptions, datasetKind]);

  // Live 100ms elapsed ticker while any GEE request runs
  useEffect(() => {
    if (busyMode === null) {
      setElapsedMs(0);
      return;
    }
    const startedAt = Date.now();
    setElapsedMs(0);
    const id = window.setInterval(() => {
      setElapsedMs(Date.now() - startedAt);
    }, 100);
    return () => {
      window.clearInterval(id);
    };
  }, [busyMode]);

  const refreshWorkbenchQuota = useCallback(async (forceRefresh = false, manualToast = false) => {
    if (forceRefresh) {
      setRefreshingWorkbenchQuota(true);
    }
    try {
      const qs = await geeQuotaStatus(forceRefresh);
      if (isValidQuotaStatus(qs)) {
        setWorkbenchQuota(qs);
        if (manualToast) {
          emitTopToast({ text: "GEE 配额已刷新", tone: "success" });
        }
      }
    } catch {
      if (manualToast) {
        emitTopToast({ text: "GEE 配额刷新失败", tone: "danger" });
      }
    } finally {
      if (forceRefresh) {
        setRefreshingWorkbenchQuota(false);
      }
    }
  }, []);

  useEffect(() => {
    void refreshWorkbenchQuota(false);
    const onCredsChanged = () => {
      void refreshWorkbenchQuota(true);
    };
    window.addEventListener("eostage:credentials-changed", onCredsChanged);
    return () => {
      window.removeEventListener("eostage:credentials-changed", onCredsChanged);
    };
  }, [refreshWorkbenchQuota]);

  const effectiveBbox: [number, number, number, number] = workingAoi
    ? [workingAoi.west, workingAoi.south, workingAoi.east, workingAoi.north]
    : [117.22, 31.8, 117.32, 31.88];

  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(() => {
      void geeCatalogSearch(searchQuery, true, 16)
        .then((res) => {
          if (!cancelled && Array.isArray(res?.items)) {
            setCatalogItems(res.items);
          }
        })
        .catch(() => {});
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [searchQuery]);

  const activeOption =
    analysisOptions.find((opt) => opt.value === analysisKey) || analysisOptions[0];

  const handleCancelActiveAction = () => {
    requestGenRef.current += 1;
    setBusyMode(null);
    setStatusNote({
      ok: false,
      text: locale === "en" ? "Current GEE operation cancelled." : "已中止当前 GEE 操作。",
    });
  };

  const handlePreflight = async () => {
    const gen = ++requestGenRef.current;
    setBusyMode("preflight");
    setStatusNote(null);
    setTooLargeError(null);
    const skipTemporal = shouldSkipGeeTemporalFilter(
      selectedAssetId,
      datasetKind,
      selectedCatalogItem
    );
    try {
      const res = (await geeCheckAvailability({
        asset_id: selectedAssetId,
        bbox: effectiveBbox,
        ...(skipTemporal
          ? {}
          : {
              start_date: startDate || "2025-04-01",
              end_date: endDate || "2025-06-30",
              max_cloud: maxCloud,
            }),
      })) as ExtendedGeeAvailability;
      if (requestGenRef.current !== gen) return;
      setAvailability(res);
      onSyncGeeResults?.({
        assetId: selectedAssetId,
        title: res.title_zh || selectedTitle,
        bbox: {
          west: effectiveBbox[0],
          south: effectiveBbox[1],
          east: effectiveBbox[2],
          north: effectiveBbox[3],
        },
        startDate: skipTemporal ? null : startDate || "2025-04-01",
        endDate: skipTemporal ? null : endDate || "2025-06-30",
        maxCloud,
        nativeScaleM: res.native_scale_m || 30,
        estimatedMb: res.grid_plan?.estimated_mb,
        totalScenes: res.total_scenes,
        matchedScenes: res.cloud_filtered_scenes,
        scenes: Array.isArray(res.scenes) ? res.scenes : [],
      });
      setStatusNote({
        ok: true,
        text: t("mapWorkbench.gee.preflightSuccess", {
          total: res.total_scenes,
          filtered: res.cloud_filtered_scenes,
          scale: res.native_scale_m,
          chunks: res.grid_plan.total_chunks,
        }),
      });
    } catch (err: unknown) {
      if (requestGenRef.current !== gen) return;
      setStatusNote({ ok: false, text: extractGeeErrorMessage(err) });
    } finally {
      if (requestGenRef.current === gen) {
        setBusyMode(null);
        void refreshWorkbenchQuota(false);
      }
    }
  };

  const runCloudMapAnalysis = async (
    mode: "add_layer" | "analyze",
    optionOverride?: GeeAnalysisOptionItem
  ) => {
    const gen = ++requestGenRef.current;
    const targetOption = optionOverride || activeOption;
    setBusyMode(mode);
    setStatusNote(null);
    setTooLargeError(null);
    try {
      const parsedThresh =
        mode === "analyze" && thresholdText.trim() !== ""
          ? Number(thresholdText)
          : undefined;
      const skipTemporal = shouldSkipGeeTemporalFilter(
        selectedAssetId,
        datasetKind,
        selectedCatalogItem
      );
      const res = await geeCloudAnalyze({
        asset_id: selectedAssetId,
        operation: targetOption.operation,
        index_name: targetOption.indexName,
        reducer: "median",
        bbox: effectiveBbox,
        ...(skipTemporal
          ? {}
          : {
              start_date: startDate || "2025-04-01",
              end_date: endDate || "2025-06-30",
              max_cloud: maxCloud,
            }),
        ...(targetOption.operation === "change_detect"
          ? { after_start_date: afterStartDate, after_end_date: afterEndDate }
          : {}),
        ...(parsedThresh !== undefined && Number.isFinite(parsedThresh)
          ? { threshold: parsedThresh, threshold_op: "gt" }
          : {}),
      });
      if (requestGenRef.current !== gen) return;

      if (typeof res?.artifact_id === "string") {
        const fallbackRecord: GeeArtifactRecord = {
          artifact_id: res.artifact_id,
          title: res.title || selectedTitle,
          dataset_id: selectedAssetId,
          operation: targetOption.operation,
          bbox: res.bbox || effectiveBbox,
          start_date: skipTemporal ? "static" : startDate || "2025-04-01",
          end_date: skipTemporal ? "static" : endDate || "2025-06-30",
          preview_data_url: res.preview_data_url,
          tile_url_template: res.tile_url_template,
          before_preview_data_url: res.before_preview_data_url,
          after_preview_data_url: res.after_preview_data_url,
          before_tile_url_template: res.before_tile_url_template,
          after_tile_url_template: res.after_tile_url_template,
          vis_min: res.vis_min ?? -1,
          vis_max: res.vis_max ?? 1,
          palette: res.palette || [],
          unit: res.unit || "",
          stats: res.stats,
          created_at: Math.floor(Date.now() / 1000),
        };
        let resolvedRecord: GeeArtifactRecord = fallbackRecord;
        try {
          const rec = await geeArtifactGet(res.artifact_id);
          if (requestGenRef.current !== gen) return;
          if (rec && typeof rec === "object" && typeof rec.artifact_id === "string") {
            resolvedRecord = {
              ...fallbackRecord,
              ...rec,
              preview_data_url: rec.preview_data_url ?? fallbackRecord.preview_data_url,
              tile_url_template: rec.tile_url_template ?? fallbackRecord.tile_url_template,
              before_preview_data_url:
                rec.before_preview_data_url ?? fallbackRecord.before_preview_data_url,
              after_preview_data_url:
                rec.after_preview_data_url ?? fallbackRecord.after_preview_data_url,
              before_tile_url_template:
                rec.before_tile_url_template ?? fallbackRecord.before_tile_url_template,
              after_tile_url_template:
                rec.after_tile_url_template ?? fallbackRecord.after_tile_url_template,
            };
          }
        } catch {
          if (requestGenRef.current !== gen) return;
        }
        pushGeeArtifactToMap(
          resolvedRecord,
          targetOption.operation === "change_detect" ? "swipe" : "single"
        );
        onSyncGeeResults?.({
          assetId: selectedAssetId,
          title: selectedTitle,
          bbox: {
            west: effectiveBbox[0],
            south: effectiveBbox[1],
            east: effectiveBbox[2],
            north: effectiveBbox[3],
          },
          startDate: skipTemporal ? null : startDate || "2025-04-01",
          endDate: skipTemporal ? null : endDate || "2025-06-30",
          maxCloud,
          nativeScaleM: availability?.native_scale_m || 30,
          estimatedMb: availability?.grid_plan?.estimated_mb,
          totalScenes: availability?.total_scenes ?? 1,
          matchedScenes: availability?.cloud_filtered_scenes ?? 1,
          scenes: availability?.scenes ?? [],
          latestArtifact: resolvedRecord,
        });
      }
      if (requestGenRef.current !== gen) return;

      if (mode === "add_layer") {
        setStatusNote({
          ok: true,
          text:
            locale === "en"
              ? `Added "${selectedTitle}" online tile preview layer to map.`
              : `已将「${selectedTitle}」在线瓦片图层叠加到地图工作台。`,
        });
      } else {
        const areaText =
          typeof res?.stats?.threshold_area_km2 === "number"
            ? t("mapWorkbench.gee.thresholdAreaSuffix", {
                area: res.stats.threshold_area_km2.toFixed(2),
              })
            : typeof res?.stats?.mean === "number"
            ? t("mapWorkbench.gee.regionMeanSuffix", {
                mean: res.stats.mean.toFixed(4),
              })
            : "";
        setStatusNote({
          ok: true,
          text:
            targetOption.operation === "change_detect"
              ? t("mapWorkbench.gee.changeDetectSuccess", { extra: areaText })
              : t("mapWorkbench.gee.analyzeSuccess", { extra: areaText }),
        });
      }
    } catch (err: unknown) {
      if (requestGenRef.current !== gen) return;
      setStatusNote({ ok: false, text: extractGeeErrorMessage(err) });
    } finally {
      if (requestGenRef.current === gen) {
        setBusyMode(null);
        void refreshWorkbenchQuota(false);
      }
    }
  };

  const handleCloudAnalyze = () => runCloudMapAnalysis("analyze");

  const handleChunkedDownload = async (opts?: {
    confirmFull?: boolean;
    usePreview?: boolean;
  }) => {
    const gen = ++requestGenRef.current;
    setBusyMode("download");
    setStatusNote(null);
    setTooLargeError(null);
    const skipTemporal = shouldSkipGeeTemporalFilter(
      selectedAssetId,
      datasetKind,
      selectedCatalogItem
    );
    try {
      const res = await geeChunkedDownload({
        asset_id: selectedAssetId,
        index_name: activeOption.indexName,
        reducer: "median",
        bbox: effectiveBbox,
        ...(skipTemporal
          ? {}
          : {
              start_date: startDate || "2025-04-01",
              end_date: endDate || "2025-06-30",
              max_cloud: maxCloud,
            }),
        confirm_full_resolution: opts?.confirmFull === true,
        use_preview_scale: opts?.usePreview === true,
      });
      if (requestGenRef.current !== gen) return;

      if (res?.quota_confirm_required === true) {
        setQuotaPrompt(res);
        return;
      }
      setQuotaPrompt(null);
      const sizeStr =
        typeof res?.bytes_written === "number" ? formatBytes(res.bytes_written) : "";
      setStatusNote({
        ok: true,
        text: t("mapWorkbench.gee.downloadSuccess", {
          chunks: res.chunks_stitched ?? 1,
          file: res.file_name || "GeoTIFF",
          size: sizeStr,
        }),
      });
    } catch (err: unknown) {
      if (requestGenRef.current !== gen) return;
      const message = extractGeeErrorMessage(err);
      setStatusNote({ ok: false, text: message });
      if (isTooLargeGridError(err, message)) {
        const errObj =
          err && typeof err === "object" ? (err as Record<string, any>) : {};
        const parsedScaleMatch = /推荐的\s*([0-9]+(?:\.[0-9]+)?)\s*米|preview_scale[^0-9]*([0-9]+)/i.exec(message);
        const parsedChunksMatch = /共需请求\s*(\d+)\s*个分块|total_chunks[^0-9]*(\d+)/i.exec(message);
        const fallbackScale = parsedScaleMatch
          ? Number(parsedScaleMatch[1] || parsedScaleMatch[2])
          : undefined;
        const fallbackChunks = parsedChunksMatch
          ? Number(parsedChunksMatch[1] || parsedChunksMatch[2])
          : undefined;
        setTooLargeError({
          message,
          suggestedScaleM:
            errObj.grid_plan?.suggested_preview_scale_m ??
            errObj.suggested_preview_scale_m ??
            availability?.grid_plan?.suggested_preview_scale_m ??
            (Number.isFinite(fallbackScale) ? fallbackScale : undefined),
          totalChunks:
            errObj.grid_plan?.total_chunks ??
            errObj.total_chunks ??
            availability?.grid_plan?.total_chunks ??
            (Number.isFinite(fallbackChunks) ? fallbackChunks : undefined),
        });
      }
    } finally {
      if (requestGenRef.current === gen) {
        setBusyMode(null);
        void refreshWorkbenchQuota(false);
      }
    }
  };

  const localizedTierLabel = workbenchQuota
    ? formatLocalizedGeeTierLabel(
        workbenchQuota.account_tier,
        workbenchQuota.account_tier_label,
        locale
      )
    : "";

  const busyActionLabel =
    busyMode === "preflight"
      ? locale === "en"
        ? "Checking scene availability..."
        : "正在预检可用景数..."
      : busyMode === "add_layer"
      ? locale === "en"
        ? "Mounting Earth Engine map tiles..."
        : "正在加载在线瓦片图层..."
      : busyMode === "analyze"
      ? locale === "en"
        ? "Running GEE cloud analysis..."
        : "正在云端计算并生成图层..."
      : busyMode === "download"
      ? locale === "en"
        ? "Downloading & stitching GeoTIFF..."
        : "正在分块下载 GeoTIFF..."
      : "";

  return (
    <div ref={geePanelRef} data-testid="gee-workbench-panel" className="flex flex-col gap-2 pt-0">
      {/* 0. Compact Live Quota & In-App Compute Strip (clean 2-line layout, zero orphan wrap) */}
      {workbenchQuota?.configured && (
        <div
          data-testid="gee-workbench-quota-strip"
          className="bg-sp-surface rounded-sp-card ring-1 ring-sp-line px-3 py-2 flex flex-col gap-1"
        >
          <div className="flex items-center justify-between gap-2 text-xs text-sp-text-secondary">
            <span className="font-semibold text-sp-text leading-snug break-words min-w-0">
              {workbenchQuota.monthly_limit_eecu_hours != null
                ? t("mapWorkbench.gee.tierLimitMonthly", {
                    tier: localizedTierLabel,
                    hours: formatQuotaHours(workbenchQuota.monthly_limit_eecu_hours),
                  })
                : localizedTierLabel}
            </span>
            <button
              type="button"
              data-testid="gee-workbench-refresh-quota-btn"
              disabled={refreshingWorkbenchQuota}
              aria-busy={refreshingWorkbenchQuota}
              onClick={() => void refreshWorkbenchQuota(true, true)}
              className="inline-flex items-center gap-1 rounded-sp-sm bg-transparent px-2 py-1 text-[13px] text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover disabled:opacity-50 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
            >
              <RefreshCw className={cn("h-3 w-3 shrink-0", quotaRefreshSpinClass)} />
              <span>{t("mapWorkbench.gee.refreshShort")}</span>
            </button>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
            <span className="font-mono text-sp-text leading-snug">
              {workbenchQuota.cloud_monitoring.unlocked &&
              workbenchQuota.cloud_monitoring.monthly_remaining_eecu_hours != null
                ? t("mapWorkbench.gee.remainingHoursShort", {
                    hours: formatQuotaHours(
                      workbenchQuota.cloud_monitoring.monthly_remaining_eecu_hours
                    ),
                  })
                : workbenchQuota.cloud_monitoring.unlocked &&
                  workbenchQuota.cloud_monitoring.monthly_used_eecu_hours != null
                ? t("mapWorkbench.gee.usedHoursShort", {
                    hours: formatQuotaHours(
                      workbenchQuota.cloud_monitoring.monthly_used_eecu_hours
                    ),
                  })
                : t("mapWorkbench.gee.inAppMonthlyUsage", {
                    requests: workbenchQuota.local_ledger.total_requests,
                    metric:
                      workbenchQuota.local_ledger.has_eecu_profile &&
                      workbenchQuota.local_ledger.profiled_eecu_seconds > 0
                        ? `${workbenchQuota.local_ledger.profiled_eecu_seconds.toFixed(1)} EECU-s`
                        : `${workbenchQuota.local_ledger.total_wall_seconds.toFixed(1)}s`,
                  })}
            </span>
            {workbenchQuota.parallelism_restricted && (
              <span className="rounded-sp-sm bg-sp-warning-bg px-2 py-1 text-xs font-semibold text-sp-warning-text whitespace-nowrap">
                {t("mapWorkbench.gee.throttledBadge")}
              </span>
            )}
          </div>
        </div>
      )}

      {/* 0.5 Active Unified Dataset Chip (linked bidirectionally with DataSourceCatalogModal) */}
      <div
        data-testid="gee-active-dataset-card"
        className="bg-sp-surface rounded-sp-card ring-1 ring-sp-line px-3 py-2 flex flex-col gap-1 shrink-0"
      >
        <div className="flex items-start justify-between gap-2 min-w-0">
          <div className="min-w-0 flex-1 flex flex-col gap-1">
            <span className="text-xs font-semibold text-sp-text leading-snug break-words">
              {selectedTitle}
            </span>
            <span
              className="font-mono text-xs text-sp-text-muted truncate"
              title={selectedAssetId}
            >
              {selectedAssetId}
            </span>
          </div>
          <span className="shrink-0 rounded-full bg-sp-surface-muted border border-sp-line px-2 py-1 text-xs font-semibold text-sp-text-secondary whitespace-nowrap">
            {datasetKindBadgeLabel(datasetKind, locale)}
          </span>
        </div>
        {onOpenCatalog && (
          <div className="pt-1 flex justify-end">
            <button
              type="button"
              data-testid="gee-open-catalog-btn"
              onClick={onOpenCatalog}
              className="inline-flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-1 text-[13px] font-semibold text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer whitespace-nowrap"
            >
              <Globe className="h-3 w-3 shrink-0" />
              <span>
                {locale === "en" ? "Browse Data Catalog (Ctrl+K)" : "从数据清单选择数据集 (Ctrl+K)"}
              </span>
            </button>
          </div>
        )}
      </div>

      {/* 1. Multilingual Dataset Search & Selection */}
      <div className="bg-sp-surface rounded-sp-card ring-1 ring-sp-line p-3 flex flex-col gap-2">
        <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
          <span className="text-xs font-semibold text-sp-text">
            {t("mapWorkbench.gee.catalogHeading")}
          </span>
          <span className="text-xs text-sp-text-muted whitespace-nowrap">
            {t("mapWorkbench.gee.catalogCountHint")}
          </span>
        </div>
        <div className="relative">
          <Search className="h-3.5 w-3.5 text-sp-text-muted absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
          <input
            type="text"
            data-testid="gee-catalog-search-input"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.currentTarget.value)}
            placeholder={t("mapWorkbench.gee.catalogSearchPlaceholder")}
            className="w-full h-8 pl-8 pr-3 rounded-sp-md border border-sp-line bg-sp-surface-muted text-[13px] text-sp-text focus:outline-none focus:ring-1 focus:ring-sp-accent"
          />
        </div>

        <div className="max-h-36 overflow-y-auto flex flex-col gap-1 pr-1">
          {catalogItems.slice(0, 8).map((item) => {
            const active = item.id === selectedAssetId;
            const displayTitle =
              locale === "zh-CN"
                ? item.title_zh || item.title_en || item.id
                : item.title_en || item.title_zh || item.id;
            const matchReasons = Array.isArray(item.match_reasons)
              ? item.match_reasons.filter(Boolean).slice(0, 2)
              : [];
            const showExtraMeta =
              Boolean(item.deprecated) ||
              matchReasons.length > 0 ||
              item.source === "community";
            return (
              <button
                key={item.id}
                type="button"
                data-testid={`gee-catalog-option-${item.id}`}
                onClick={() => selectAssetUnified(item.id, item)}
                className={cn(
                  "w-full text-left px-3 py-2 rounded-sp-md border text-[13px] transition-colors cursor-pointer flex flex-col gap-1",
                  active
                    ? "border-sp-text bg-sp-selected text-sp-text"
                    : "border-sp-line bg-sp-surface hover:bg-sp-hover text-sp-text-secondary"
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-semibold truncate text-sp-text">{displayTitle}</span>
                  <span
                    className={cn(
                      "shrink-0 px-2 py-0 rounded-sp-sm text-xs whitespace-nowrap",
                      item.source === "community"
                        ? "bg-sp-warning-bg text-sp-warning-text"
                        : "bg-sp-surface-muted text-sp-text-muted"
                    )}
                  >
                    {item.source === "community"
                      ? t("mapWorkbench.gee.communityTag")
                      : t("mapWorkbench.gee.officialTag")}
                    {item.native_scale_m ? ` · ${item.native_scale_m}m` : ""}
                  </span>
                </div>
                <span className="font-mono text-xs text-sp-text-muted truncate">{item.id}</span>
                {showExtraMeta && (
                  <div className="flex flex-wrap items-center gap-1 pt-1">
                    {item.deprecated && (
                      <span className="px-2 py-0 rounded-sp-sm text-xs bg-sp-danger-bg text-sp-danger-text">
                        {t("mapWorkbench.gee.deprecatedBadge")}
                      </span>
                    )}
                    {item.deprecated && item.replacement_id && (
                      <span className="text-xs text-sp-danger-text truncate">
                        {t("mapWorkbench.gee.deprecatedReplacementHint", {
                          id: item.replacement_id,
                        })}
                      </span>
                    )}
                    {matchReasons.map((reason, rIdx) => (
                      <span
                        key={rIdx}
                        className="px-2 py-0 rounded-sp-sm text-xs bg-sp-surface-muted text-sp-text-secondary"
                      >
                        {reason}
                      </span>
                    ))}
                    {item.source === "community" && (
                      <span className="text-xs text-sp-warning-text">
                        {t("mapWorkbench.gee.communityVerifyHint")}
                      </span>
                    )}
                  </div>
                )}
              </button>
            );
          })}
        </div>

        <div className="flex flex-col gap-1 pt-1 border-t border-sp-line">
          <label className="text-[13px] text-sp-text-muted">
            {t("mapWorkbench.gee.selectedAssetLabel")}
          </label>
          <input
            type="text"
            aria-label={t("mapWorkbench.gee.selectedAssetAria")}
            value={selectedAssetId}
            onChange={(e) => selectAssetUnified(e.currentTarget.value)}
            className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface font-mono text-[13px] text-sp-text"
          />
        </div>
      </div>

      {/* 2. Adaptive Cloud Composite / Index / Time & Cloud Controls */}
      <div className="bg-sp-surface rounded-sp-card ring-1 ring-sp-line p-3 flex flex-col gap-3">
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs font-semibold text-sp-text">
            {t("mapWorkbench.gee.computeSettingsHeading")}
          </span>
          <span className="text-xs text-sp-text-muted whitespace-nowrap">
            {datasetKindBadgeLabel(datasetKind, locale)}
          </span>
        </div>
        <SpSelect
          aria-label={t("mapWorkbench.gee.operatorSelectAria")}
          data-testid="gee-analysis-mode-select"
          value={activeOption?.value || analysisKey}
          onChange={setAnalysisKey}
          size="sm"
          options={analysisOptions.map((opt) => ({
            value: opt.value,
            label: opt.label,
          }))}
        />

        {!shouldSkipGeeTemporalFilter(selectedAssetId, datasetKind, selectedCatalogItem) ? (
          <div className="grid grid-cols-2 gap-2">
            <div className="flex flex-col gap-1">
              <span className="text-xs text-sp-text-muted">
                {activeOption.operation === "change_detect"
                  ? t("mapWorkbench.gee.beforeStart")
                  : t("mapWorkbench.gee.startDate")}
              </span>
              <input
                type="date"
                value={startDate || "2025-04-01"}
                onChange={(e) => onStartDateChange(e.currentTarget.value)}
                className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] font-mono text-sp-text"
              />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-xs text-sp-text-muted">
                {activeOption.operation === "change_detect"
                  ? t("mapWorkbench.gee.beforeEnd")
                  : t("mapWorkbench.gee.endDate")}
              </span>
              <input
                type="date"
                value={endDate || "2025-06-30"}
                onChange={(e) => onEndDateChange(e.currentTarget.value)}
                className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] font-mono text-sp-text"
              />
            </div>
          </div>
        ) : (
          <div
            data-testid="gee-timeless-dataset-note"
            className="rounded-sp-sm bg-sp-surface-muted px-3 py-1 text-xs text-sp-text-secondary leading-snug"
          >
            {locale === "en"
              ? "Static elevation / baseline layer: date & cloud filters are automatically skipped."
              : "静态高程 / 专题基准数据集：已自动跳过日期窗口与云量筛选。"}
          </div>
        )}

        {activeOption.operation === "change_detect" && (
          <div className="grid grid-cols-2 gap-2">
            <div className="flex flex-col gap-1">
              <span className="text-xs text-sp-text-muted">
                {t("mapWorkbench.gee.afterStart")}
              </span>
              <input
                type="date"
                value={afterStartDate}
                onChange={(e) => setAfterStartDate(e.currentTarget.value)}
                className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] font-mono text-sp-text"
              />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-xs text-sp-text-muted">
                {t("mapWorkbench.gee.afterEnd")}
              </span>
              <input
                type="date"
                value={afterEndDate}
                onChange={(e) => setAfterEndDate(e.currentTarget.value)}
                className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] font-mono text-sp-text"
              />
            </div>
          </div>
        )}

        <div className="grid grid-cols-2 gap-2 items-end">
          {(datasetKind === "optical" ||
            datasetKind === "climate" ||
            datasetKind === "generic") && (
            <div className="flex flex-col gap-1">
              <span className="text-xs text-sp-text-muted">
                {t("mapWorkbench.gee.maxCloudLabel", { cloud: maxCloud })}
              </span>
              <input
                type="range"
                min={0}
                max={100}
                value={maxCloud}
                onChange={(e) => setMaxCloud(Number(e.currentTarget.value))}
                className="accent-sp-accent cursor-pointer"
              />
            </div>
          )}
          <div
            className={cn(
              "flex flex-col gap-1",
              datasetKind !== "optical" &&
                datasetKind !== "climate" &&
                datasetKind !== "generic" &&
                "col-span-2"
            )}
          >
            <span className="text-xs text-sp-text-muted">
              {t("mapWorkbench.gee.areaThresholdLabel")}
            </span>
            <input
              type="number"
              step="0.05"
              placeholder={t("mapWorkbench.gee.areaThresholdPlaceholder")}
              value={thresholdText}
              onChange={(e) => setThresholdText(e.currentTarget.value)}
              className="h-7 px-2 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] font-mono text-sp-text"
            />
          </div>
        </div>

        <div className="text-xs text-sp-text-muted leading-snug">
          {workingAoi
            ? t("mapWorkbench.gee.currentExtentSelected", {
                aoi: aoiLabel || t("mapWorkbench.gee.drawnAoiFallback"),
              })
            : t("mapWorkbench.gee.defaultHefeiExtent")}
        </div>
      </div>

      {/* 3. Quota Guardrail Prompt (> 9 chunks) */}
      {quotaPrompt && (
        <div
          data-testid="gee-workbench-quota-card"
          className="rounded-sp-card border border-sp-warning-text/40 bg-sp-warning-bg p-3 flex flex-col gap-2 text-xs"
        >
          <div className="flex items-center gap-2 font-semibold text-sp-warning-text">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
            <span>
              {t("mapWorkbench.gee.largeGridConfirmHeading", {
                chunks: quotaPrompt.grid_plan?.total_chunks ?? 0,
              })}
            </span>
          </div>
          <p className="text-sp-text-secondary leading-snug">
            {locale === "zh-CN" && quotaPrompt.message_zh
              ? quotaPrompt.message_zh
              : t("agent.gee.quotaGuardrailFallback", {
                  scale: quotaPrompt.grid_plan?.requested_scale_m ?? 10,
                  chunks: quotaPrompt.grid_plan?.total_chunks ?? 12,
                  width: quotaPrompt.grid_plan?.full_width ?? 0,
                  height: quotaPrompt.grid_plan?.full_height ?? 0,
                  mb: Number(quotaPrompt.grid_plan?.estimated_mb ?? 0).toFixed(1),
                })}
          </p>
          <div className="flex flex-col gap-2">
            <button
              type="button"
              data-testid="gee-workbench-confirm-full-btn"
              disabled={busyMode !== null}
              onClick={() => void handleChunkedDownload({ confirmFull: true })}
              className="h-7 px-3 rounded-sp-md bg-sp-accent-fill text-white text-[13px] font-semibold cursor-pointer"
            >
              {t("mapWorkbench.gee.confirmFullResDownload", {
                scale: quotaPrompt.grid_plan?.requested_scale_m ?? 10,
              })}
            </button>
            <button
              type="button"
              data-testid="gee-workbench-confirm-preview-btn"
              disabled={busyMode !== null}
              onClick={() => void handleChunkedDownload({ usePreview: true })}
              className="h-7 px-3 rounded-sp-md border border-sp-line bg-sp-surface text-[13px] text-sp-text cursor-pointer"
            >
              {t("mapWorkbench.gee.confirmPreviewDownload", {
                scale: quotaPrompt.grid_plan?.suggested_preview_scale_m ?? 30,
              })}
            </button>
          </div>
        </div>
      )}

      {/* 3.5 Over-64 Chunk Recovery Banner */}
      {tooLargeError && (
        <div
          data-testid="gee-too-large-recovery-card"
          className="rounded-sp-card border border-sp-line bg-sp-surface-muted p-3 flex flex-col gap-2 text-xs"
        >
          <div className="flex items-start gap-2 font-semibold text-sp-text">
            <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-1 text-sp-text-secondary" />
            <span className="leading-snug">
              {tooLargeError.totalChunks
                ? `当前范围需 ${tooLargeError.totalChunks} 块（超出单次免 Drive 分块上限 64 块）`
                : "研究区网格超出单次免 Drive 分块下载上限（64 块）"}
            </span>
          </div>
          <p className="text-sp-text-secondary leading-relaxed m-0">
            {tooLargeError.suggestedScaleM
              ? `推荐使用 ${tooLargeError.suggestedScaleM}m 预览分辨率一键完成 GeoTIFF 下载，或在地图上缩小研究区后重试（也可直接使用上方「添加到地图图层」在线浏览）。`
              : "推荐使用预览分辨率降采样下载，或在地图上缩小研究区后重试。"}
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              data-testid="gee-retry-preview-scale-btn"
              disabled={busyMode !== null}
              onClick={() => void handleChunkedDownload({ usePreview: true })}
              className="h-7 px-3 rounded-sp-sm bg-sp-accent-fill text-sp-accent-ink text-[13px] font-semibold hover:opacity-90 disabled:opacity-50 cursor-pointer whitespace-nowrap"
            >
              {tooLargeError.suggestedScaleM
                ? `按推荐预览分辨率下载 (${tooLargeError.suggestedScaleM}m)`
                : "按推荐预览分辨率下载"}
            </button>
            <button
              type="button"
              data-testid="gee-narrow-aoi-dismiss-btn"
              onClick={() => setTooLargeError(null)}
              className="h-7 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-[13px] text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer whitespace-nowrap"
            >
              缩小研究区后重试
            </button>
          </div>
        </div>
      )}

      {/* 4. Strict 2-Row 3-Step Progression: Step 1 Preflight -> Step 2a Analyze & Step 2b Download */}
      <div className="bg-sp-surface rounded-sp-card ring-1 ring-sp-line p-3 flex flex-col gap-2">
        <button
          type="button"
          data-testid="gee-preflight-btn"
          disabled={busyMode !== null}
          onClick={() => void handlePreflight()}
          className="w-full h-8 px-3 rounded-sp-md border border-sp-line bg-sp-surface text-[13px] font-semibold text-sp-text hover:bg-sp-hover disabled:opacity-50 inline-flex items-center justify-center gap-2 cursor-pointer whitespace-nowrap"
        >
          {busyMode === "preflight" && showGeeActionSpinner ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
          ) : (
            <Search className="h-3.5 w-3.5 text-sp-text-secondary shrink-0" />
          )}
          <span>{t("mapWorkbench.gee.preflightBtn")}</span>
        </button>

        <div className="grid grid-cols-2 gap-2">
          <button
            type="button"
            data-testid="gee-workbench-analyze-btn"
            disabled={busyMode !== null || !preflightPassed}
            aria-busy={busyMode === "analyze"}
            title={
              !preflightPassed
                ? locale === "en"
                  ? "Run preflight check first to confirm available scenes"
                  : "请先检索并确认当前研究区存在可用影像后再计算叠加"
                : undefined
            }
            onClick={() => void handleCloudAnalyze()}
            className="min-w-0 overflow-hidden h-8 px-2 rounded-sp-md bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 disabled:opacity-50 inline-flex items-center justify-center gap-1 cursor-pointer whitespace-nowrap"
          >
            {busyMode === "analyze" && showGeeActionSpinner ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
            ) : (
              <Layers className="h-3.5 w-3.5 shrink-0" />
            )}
            <span className="truncate">{t("mapWorkbench.gee.analyzeToMapBtn")}</span>
          </button>

          <button
            type="button"
            data-testid="gee-workbench-download-btn"
            disabled={busyMode !== null || !preflightPassed}
            aria-busy={busyMode === "download"}
            title={
              !preflightPassed
                ? locale === "en"
                  ? "Run preflight check first to confirm available scenes"
                  : "请先检索并确认当前研究区存在可用影像后再下载 GeoTIFF"
                : undefined
            }
            onClick={() => void handleChunkedDownload()}
            className="min-w-0 overflow-hidden h-8 px-2 rounded-sp-md bg-sp-accent-fill text-white text-[13px] font-semibold hover:opacity-90 disabled:opacity-50 inline-flex items-center justify-center gap-1 cursor-pointer whitespace-nowrap"
          >
            {busyMode === "download" && showGeeActionSpinner ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" />
            ) : (
              <Download className="h-3.5 w-3.5 shrink-0" />
            )}
            <span className="truncate">{t("mapWorkbench.gee.chunkedDownloadBtn")}</span>
          </button>
        </div>

        {/* Active Request Progress Bar + Elapsed Timer (>1.0s) + Cancel */}
        {busyMode !== null && (
          <div
            data-testid="gee-action-progress-bar"
            className="flex items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-3 py-1 text-xs"
          >
            <div className="min-w-0 flex items-center gap-2 text-sp-text-secondary">
              <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0 text-sp-text" />
              <span className="truncate">{busyActionLabel}</span>
              {elapsedMs >= 1000 && (
                <span
                  data-testid="gee-action-elapsed-timer"
                  className="font-mono text-sp-text tabular-nums whitespace-nowrap shrink-0"
                >
                  {locale === "en"
                    ? `${(elapsedMs / 1000).toFixed(1)}s`
                    : `已用时 ${(elapsedMs / 1000).toFixed(1)}s`}
                </span>
              )}
            </div>
            <button
              type="button"
              data-testid="gee-action-cancel-btn"
              onClick={handleCancelActiveAction}
              className="inline-flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-1 text-[13px] font-semibold text-sp-text hover:bg-sp-hover cursor-pointer shrink-0 whitespace-nowrap"
            >
              <X className="h-3 w-3 shrink-0" />
              <span>{locale === "en" ? "Cancel" : "中止"}</span>
            </button>
          </div>
        )}

        {/* Structured Preflight Results Card */}
        {availability && (
          <div
            data-testid="gee-preflight-card"
            className="rounded-sp-md border border-sp-line bg-sp-surface-muted p-3 flex flex-col gap-1 text-xs"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="font-semibold text-sp-text">
                {locale === "en"
                  ? `${availability.cloud_filtered_scenes} / ${availability.total_scenes} scenes ready`
                  : `可用 ${availability.cloud_filtered_scenes} / 总计 ${availability.total_scenes} 景`}
              </span>
              <span className="font-mono text-sp-text-muted whitespace-nowrap">
                {availability.native_scale_m}m · {availability.grid_plan.total_chunks} 块
              </span>
            </div>
            <div className="flex flex-col gap-1 text-xs text-sp-text-secondary">
              {(availability.start_date || startDate) && (
                <span>
                  时间窗：{availability.start_date || startDate || "2025-04-01"} ~{" "}
                  {availability.end_date || endDate || "2025-06-30"}
                </span>
              )}
              {Array.isArray(availability.bands) && availability.bands.length > 0 && (
                <span className="font-mono break-all">
                  波段 ({availability.bands.length})：{availability.bands.slice(0, 8).join(", ")}
                  {availability.bands.length > 8 ? " …" : ""}
                </span>
              )}
              <span>
                网格估计：{availability.grid_plan.full_width}×{availability.grid_plan.full_height} px
                {Number.isFinite(availability.grid_plan.estimated_mb)
                  ? ` · 约 ${availability.grid_plan.estimated_mb.toFixed(1)} MB`
                  : ""}
              </span>
            </div>
          </div>
        )}

        {statusNote && (
          <div
            role="status"
            data-testid="gee-workbench-status-note"
            className={cn(
              "rounded-sp-md px-3 py-2 text-xs leading-snug break-words",
              statusNote.ok
                ? "bg-sp-surface-muted text-sp-text"
                : "bg-sp-danger-bg text-sp-danger-text"
            )}
          >
            {statusNote.text}
          </div>
        )}

        <div
          data-testid="gee-custom-agent-hint"
          className="rounded-sp-sm border border-sp-line/70 bg-sp-surface-muted px-3 py-2 text-xs text-sp-text-secondary leading-relaxed"
        >
          {locale === "en"
            ? "This panel provides zero-code preset composites and spectral indices. For custom band formulas, adaptive thresholding, or classification scripts, describe your algorithm in Agent Chat and confirm the inspected GEE Python code card to run it."
            : "当前面板提供零代码预设合成与指数计算；如需自定义波段公式、自适应阈值或分类脚本，可在「对话」界面直接描述算法，由智能体自动编写并经你确认后运行 GEE 代码。"}
        </div>
      </div>
    </div>
  );
}
