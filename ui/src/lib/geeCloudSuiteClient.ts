import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import { getCurrentLocale, t, translate } from "@/i18n";

export const GEE_QUOTA_CONFIGURATION_URL =
  "https://console.cloud.google.com/earth-engine/configuration";

export const GEE_SERVICE_ACCOUNT_CONSOLE_URL =
  "https://console.cloud.google.com/iam-admin/serviceaccounts";

export function formatLocalizedGeeTierLabel(
  accountTier: string | undefined | null,
  fallbackLabel: string | undefined | null,
  locale = getCurrentLocale()
): string {
  if (locale === "zh-CN" && fallbackLabel) {
    return fallbackLabel;
  }
  switch (accountTier) {
    case "noncommercial_community":
      return translate(locale, "accounts.gee.tierCommunity");
    case "noncommercial_contributor":
      return translate(locale, "accounts.gee.tierContributor");
    case "noncommercial_partner":
      return translate(locale, "accounts.gee.tierPartner");
    case "noncommercial_unlimited":
    case "noncommercial_unknown":
      return translate(locale, "accounts.gee.tierNoncommercialGeneric");
    case "commercial":
      return translate(locale, "accounts.gee.tierCommercial");
    case "unknown":
      return translate(locale, "accounts.gee.tierConfiguredUnknown");
    case "unconfigured":
      return translate(locale, "accounts.gee.tierUnconfigured");
    default:
      return locale === "zh-CN" && fallbackLabel
        ? fallbackLabel
        : translate(locale, "accounts.gee.tierUnconfigured");
  }
}

export interface GeeAccountStatus {
  ok: boolean;
  gee_ready: boolean;
  project_id?: string | null;
  auth_mode?: string | null;
  has_private_key?: boolean;
}

export interface GeeCloudMonitoringQuota {
  unlocked: boolean;
  reason?: string | null;
  monthly_used_eecu_seconds?: number | null;
  monthly_used_eecu_hours?: number | null;
  monthly_remaining_eecu_hours?: number | null;
}

export interface GeeLocalUsageLedger {
  month_utc: string;
  total_requests: number;
  analysis_runs: number;
  chunk_downloads: number;
  script_runs: number;
  profiled_eecu_seconds: number;
  has_eecu_profile: boolean;
  total_wall_seconds: number;
  last_call_eecu_seconds?: number | null;
  last_call_wall_seconds?: number | null;
  last_updated_at_ms?: number | null;
}

export interface GeeQuotaStatusResponse {
  ok: boolean;
  configured: boolean;
  project_id?: string | null;
  registration_state?: string | null;
  account_tier: string;
  account_tier_label: string;
  monthly_limit_eecu_seconds?: number | null;
  monthly_limit_eecu_hours?: number | null;
  daily_limit_eecu_seconds?: number | null;
  read_requests_per_minute?: number | null;
  parallelism_restricted: boolean;
  last_quota_status_header?: string | null;
  cloud_monitoring: GeeCloudMonitoringQuota;
  local_ledger: GeeLocalUsageLedger;
  fetched_at_ms: number;
}

export type GeeQuotaStatus = GeeQuotaStatusResponse;

export interface GeeCatalogItem {
  id: string;
  title?: string;
  title_zh: string;
  title_en: string;
  source: "official" | "community" | string;
  asset_type: "image_collection" | "image" | string;
  provider?: string;
  native_scale_m?: number;
  start_year?: string;
  end_year?: string;
  default_bands?: string[];
  supported_indices?: string[];
  tags?: string[] | string;
  deprecated?: boolean;
  replacement_id?: string | null;
  score?: number;
  confidence?: "high" | "medium" | "low" | string;
  match_reasons?: string[];
  verification_rule?: string;
}

export function normalizeGeeCatalogItem(raw: any): GeeCatalogItem {
  const tagsRaw = raw?.tags;
  const tags: string[] = Array.isArray(tagsRaw)
    ? tagsRaw.map((v) => String(v ?? "").trim()).filter(Boolean)
    : typeof tagsRaw === "string"
      ? tagsRaw
          .split(/[\s,;，；]+/)
          .map((v) => v.trim())
          .filter(Boolean)
      : [];
  const id = String(raw?.id ?? "").trim();
  const titleFallback = String(raw?.title_en ?? raw?.title ?? id).trim();
  return {
    ...raw,
    id,
    title_zh: String(raw?.title_zh ?? titleFallback).trim(),
    title_en: titleFallback,
    source: String(raw?.source ?? "official"),
    asset_type: String(raw?.asset_type ?? raw?.type ?? "image_collection"),
    default_bands: Array.isArray(raw?.default_bands)
      ? raw.default_bands.map((b: unknown) => String(b))
      : [],
    tags,
  };
}

export interface ChunkGridPlan {
  bbox: [number, number, number, number];
  requested_scale_m: number;
  effective_scale_m: number;
  suggested_preview_scale_m: number;
  scale_x_deg: number;
  scale_y_deg: number;
  full_width: number;
  full_height: number;
  chunk_size_px: number;
  chunks_x: number;
  chunks_y: number;
  total_chunks: number;
  estimated_mb: number;
  quota_confirm_required: boolean;
}

export interface GeeSceneSummary {
  id: string;
  image_id: string;
  acquisition_date?: string | null;
  cloud_cover?: number | null;
}

export interface GeeAvailabilityResult {
  ok: boolean;
  asset_id: string;
  title_zh: string;
  asset_type: string;
  total_scenes: number;
  cloud_filtered_scenes: number;
  native_scale_m: number;
  grid_plan: ChunkGridPlan;
  scenes?: GeeSceneSummary[];
}

export interface GeeTimeSeriesWindow {
  window_start: string;
  window_end: string;
  scene_count: number;
  mean?: number | null;
  min?: number | null;
  max?: number | null;
  stats?: {
    mean?: number | null;
    min?: number | null;
    max?: number | null;
  } | null;
}

export interface GeeDataLayerRecord {
  target_variable?: string;
  dataset_source: "official" | "community" | "unknown" | string;
  dataset_id: string;
  asset_type?: string;
  deprecated?: boolean;
  replacement_id?: string | null;
  time_range?: {
    start_date?: string | null;
    end_date?: string | null;
    is_timeless?: boolean;
  };
  temporal_reducer?: string | null;
  spatial_extent_source?: string;
  bbox?: [number, number, number, number] | null;
  native_scale_m?: number | null;
  analysis_scale_m?: number | null;
  crs?: string;
  bands?: string[];
  scale_offset_note?: string;
  unit?: string;
  qa_mask_method?: string;
  output_target?: string;
  verification_status?: string;
}

export interface GeeScriptReviewFinding {
  severity: "ERROR" | "WARN" | "INFO" | string;
  code: string;
  line: number;
  message: string;
}

export interface GeeExportPreflightResult {
  ok: boolean;
  ready: boolean;
  asset_id: string;
  scale_m: number;
  algorithm_kind: string;
  tile_safety: "tile_safe" | "tile_unsafe" | string;
  tile_safety_reason?: string;
  destination?: string;
  recommended_route:
    | "local_chunked_geotiff_auto"
    | "local_chunked_geotiff_confirm"
    | "external_batch_export"
    | "needs_parameters"
    | string;
  quota_confirm_required?: boolean;
  raw_width_px: number;
  raw_height_px: number;
  raw_total_pixels?: number;
  chunks_x?: number;
  chunks_y?: number;
  raw_total_chunks: number;
  estimated_mb: number;
  grid_plan?: ChunkGridPlan | null;
  missing_parameters?: string[];
  warnings?: string[];
  message_zh?: string;
  data_layer_record?: GeeDataLayerRecord | null;
}

export interface GeeRecommendResult {
  ok: boolean;
  task: string;
  recipe?: string | null;
  recipe_title_zh?: string;
  roles: Record<string, { label_zh?: string; candidates: GeeCatalogItem[] }>;
  missing_roles?: string[];
  needs_clarification?: boolean;
  clarification_zh?: string | null;
  selection_note_zh?: string;
}

export interface GeeCompareResult {
  ok: boolean;
  requested: string[];
  datasets: GeeCatalogItem[];
  missing: string[];
  comparison_fields?: string[];
  error?: string;
  message_zh?: string;
}

export interface GeeVerifyResult {
  ok: boolean;
  id: string;
  source: string;
  status: "verified" | "catalog_only" | "deprecated" | "not_found" | string;
  deprecated: boolean;
  catalog_found: boolean;
  catalog_record?: GeeCatalogItem | null;
  stac_checked?: boolean;
  stac_found?: boolean;
  stac_scale_m?: number | null;
  stac_band_names?: string[];
  live_checked?: boolean;
  live_exists?: boolean | null;
  live_status?: string;
  verification_rule?: string;
}

export interface GeeArtifactRecord {
  artifact_id: string;
  title: string;
  dataset_id: string;
  operation: string;
  bbox: [number, number, number, number];
  start_date?: string | null;
  end_date?: string | null;
  preview_data_url?: string | null;
  before_preview_data_url?: string | null;
  after_preview_data_url?: string | null;
  tile_url_template?: string | null;
  before_tile_url_template?: string | null;
  after_tile_url_template?: string | null;
  output_path?: string | null;
  size_bytes?: number | null;
  vis_min: number;
  vis_max: number;
  palette: string[];
  unit: string;
  series?: GeeTimeSeriesWindow[];
  stats?: {
    series?: GeeTimeSeriesWindow[];
    [key: string]: any;
  };
  data_layer_record?: GeeDataLayerRecord | null;
  created_at: number;
}

export interface GeeCloudAnalyzeRequest {
  asset_id: string;
  operation: "composite" | "index" | "zonal_stats" | "change_detect" | "time_series";
  index_name?: string;
  reducer?: "median" | "mean" | "min" | "max" | "mosaic";
  bands?: string[];
  bbox?: [number, number, number, number];
  start_date?: string;
  end_date?: string;
  after_start_date?: string;
  after_end_date?: string;
  max_cloud?: number;
  scale_m?: number;
  threshold?: number;
  threshold_op?: "gt" | "lt";
  step_days?: number;
}

export interface GeeChunkedDownloadRequest {
  asset_id: string;
  index_name?: string;
  bands?: string[];
  reducer?: "median" | "mean" | "min" | "max" | "mosaic";
  bbox?: [number, number, number, number];
  start_date?: string;
  end_date?: string;
  max_cloud?: number;
  scale_m?: number;
  output_path?: string;
  confirm_full_resolution?: boolean;
  use_preview_scale?: boolean;
}

export interface GeeScriptInspectReport {
  ok: boolean;
  blocked_reasons: string[];
  scientific_warnings: string[];
  findings?: GeeScriptReviewFinding[];
  approval_id?: string | null;
  script_sha256: string;
  env_ready: boolean;
}

export type GeeQueryInvoker = (request: Record<string, any>) => Promise<any>;

let testOverrideInvoker: GeeQueryInvoker | null = null;

export function setGeeQueryInvokerForTest(invoker: GeeQueryInvoker | null): void {
  testOverrideInvoker = invoker;
}

export async function invokeGeeQuery<T = any>(
  dataset: string,
  geeParams: Record<string, any> = {},
  topLevel: {
    keyword?: string;
    bbox?: [number, number, number, number];
    start_date?: string;
    end_date?: string;
    output_path?: string;
  } = {}
): Promise<T> {
  const payload = {
    dataset,
    keyword: topLevel.keyword,
    bbox: topLevel.bbox ?? geeParams.bbox,
    start_date: topLevel.start_date ?? geeParams.start_date,
    end_date: topLevel.end_date ?? geeParams.end_date,
    output_path: topLevel.output_path ?? geeParams.output_path,
    gee_params: geeParams,
  };
  if (testOverrideInvoker) {
    return (await testOverrideInvoker(payload)) as T;
  }
  const bridge = getDesktopShellBridge();
  const fn = bridge?.application?.openGeodataQuery;
  if (typeof fn !== "function") {
    throw new Error(t("mapWorkbench.gee.bridgeUnavailable"));
  }
  const res = await fn(payload);
  if (res && typeof res === "object" && res.ok === false && res.error) {
    const msg =
      typeof res.error === "string"
        ? res.error
        : res.error?.message || JSON.stringify(res.error);
    throw new Error(msg);
  }
  return res as T;
}

export function geeAccountStatus(): Promise<GeeAccountStatus> {
  return invokeGeeQuery<GeeAccountStatus>("gee_account_status");
}

export function geeAccountSave(projectId: string, secret: string): Promise<any> {
  return invokeGeeQuery("gee_account_save", { project_id: projectId, secret });
}

export function geeAccountVerify(projectId?: string, secret?: string): Promise<any> {
  return invokeGeeQuery("gee_account_verify", {
    ...(projectId ? { project_id: projectId } : {}),
    ...(secret ? { secret } : {}),
  });
}

export function geeAccountClear(): Promise<any> {
  return invokeGeeQuery("gee_account_clear");
}

export function geeQuotaStatus(refresh = false): Promise<GeeQuotaStatusResponse> {
  return invokeGeeQuery<GeeQuotaStatusResponse>("gee_quota_status", { refresh });
}

export async function geeCatalogSearch(
  query: string,
  includeCommunity = true,
  limit = 20,
  forceRefresh = false,
  filters?: {
    source?: "official" | "community" | "curated";
    provider?: string;
    kind?: "image_collection" | "image" | "table";
    max_resolution_m?: number;
    include_deprecated?: boolean;
  }
): Promise<{
  ok: boolean;
  items: GeeCatalogItem[];
  official_total?: number;
  community_total?: number;
  detected_concepts?: string[];
  needs_clarification?: boolean;
  clarification_zh?: string | null;
}> {
  const res = await invokeGeeQuery<{
    ok: boolean;
    items?: unknown[];
    official_total?: number;
    community_total?: number;
    detected_concepts?: string[];
    needs_clarification?: boolean;
    clarification_zh?: string | null;
  }>("gee_catalog_search", {
    query,
    include_community: includeCommunity,
    limit,
    force_refresh: forceRefresh,
    ...(filters ?? {}),
  });
  return {
    ...res,
    items: Array.isArray(res?.items) ? res.items.map(normalizeGeeCatalogItem) : [],
  };
}

export async function geeRecommendDatasets(
  task: string,
  limitPerRole = 2,
  includeCommunity = true
): Promise<GeeRecommendResult> {
  const res = await invokeGeeQuery<any>("gee_recommend_datasets", {
    task,
    limit_per_role: limitPerRole,
    include_community: includeCommunity,
  });
  const roles: GeeRecommendResult["roles"] = {};
  if (res?.roles && typeof res.roles === "object") {
    for (const [key, val] of Object.entries<any>(res.roles)) {
      roles[key] = {
        label_zh: val?.label_zh,
        candidates: Array.isArray(val?.candidates)
          ? val.candidates.map(normalizeGeeCatalogItem)
          : [],
      };
    }
  }
  return {
    ...res,
    roles,
  };
}

export async function geeCompareDatasets(
  ids: string[],
  includeCommunity = true
): Promise<GeeCompareResult> {
  const res = await invokeGeeQuery<any>("gee_compare_datasets", {
    ids,
    include_community: includeCommunity,
  });
  return {
    ...res,
    datasets: Array.isArray(res?.datasets)
      ? res.datasets.map(normalizeGeeCatalogItem)
      : [],
  };
}

export function geeVerifyDataset(
  assetId: string,
  live = false
): Promise<GeeVerifyResult> {
  return invokeGeeQuery<GeeVerifyResult>("gee_verify_dataset", {
    asset_id: assetId,
    live,
  });
}

export function geeExportPreflight(params: {
  asset_id: string;
  bbox?: [number, number, number, number];
  scale_m?: number;
  bands?: string[];
  index_name?: string;
  algorithm_kind?: string;
  destination?: "local" | "drive" | "cloud_storage" | "ee_asset";
  start_date?: string;
  end_date?: string;
  max_cloud?: number;
  reducer?: string;
  qa_mask?: string;
  target_variable?: string;
  crs?: string;
}): Promise<GeeExportPreflightResult> {
  return invokeGeeQuery<GeeExportPreflightResult>("gee_export_preflight", params);
}

export function geeDatasetBands(assetId: string): Promise<any> {
  return invokeGeeQuery("gee_dataset_bands", { asset_id: assetId });
}

export function geeCheckAvailability(params: {
  asset_id: string;
  bbox?: [number, number, number, number];
  start_date?: string;
  end_date?: string;
  max_cloud?: number;
  scale_m?: number;
}): Promise<GeeAvailabilityResult> {
  return invokeGeeQuery<GeeAvailabilityResult>("gee_check_availability", params);
}

export function geeCloudAnalyze(params: GeeCloudAnalyzeRequest): Promise<any> {
  return invokeGeeQuery("gee_cloud_analyze", params);
}

export function geeChunkedDownload(params: GeeChunkedDownloadRequest): Promise<any> {
  return invokeGeeQuery("gee_chunked_download", params);
}

export function geeArtifactGet(artifactId: string): Promise<GeeArtifactRecord> {
  return invokeGeeQuery<GeeArtifactRecord>("gee_artifact_get", { artifact_id: artifactId });
}

export function geeScriptInspect(
  script: string,
  bbox?: [number, number, number, number]
): Promise<GeeScriptInspectReport> {
  return invokeGeeQuery<GeeScriptInspectReport>("gee_script_inspect", { script, bbox });
}

export function geeScriptPreview(
  approvalId: string
): Promise<{ ok: boolean; approval_id: string; script: string; script_sha256: string }> {
  return invokeGeeQuery("gee_script_preview", { approval_id: approvalId });
}

export function geeScriptExecute(
  approvalId: string,
  script: string,
  bbox?: [number, number, number, number]
): Promise<any> {
  return invokeGeeQuery("gee_script_execute", {
    approval_id: approvalId,
    script,
    bbox,
  });
}

// ============================================================================
// Shared Multi-Layer Stack & Split-Screen Swipe State Store (`WorkbenchMap`)
// ============================================================================

export interface GeeMapLayerItem {
  id: string;
  title: string;
  datasetId: string;
  operation: string;
  bbox: [number, number, number, number];
  visible: boolean;
  opacity: number;
  pinned: boolean;
  tileUrlTemplate?: string | null;
  previewDataUrl?: string | null;
  visMin: number;
  visMax: number;
  palette: string[];
  unit: string;
  stats?: Record<string, any>;
}

export interface GeeSwipeState {
  enabled: boolean;
  positionPct: number;
  leftLayerId: string | null;
  rightLayerId: string | null;
}

export interface GeeLayerStoreSnapshot {
  layers: GeeMapLayerItem[];
  swipe: GeeSwipeState;
  zoomTarget: { bbox: [number, number, number, number]; nonce: number } | null;
}

let storeState: GeeLayerStoreSnapshot = {
  layers: [],
  swipe: {
    enabled: false,
    positionPct: 50,
    leftLayerId: null,
    rightLayerId: null,
  },
  zoomTarget: null,
};

const listeners = new Set<() => void>();

function emitChange() {
  for (const listener of listeners) {
    listener();
  }
}

export function getGeeLayerStoreSnapshot(): GeeLayerStoreSnapshot {
  return storeState;
}

export function subscribeGeeLayerStore(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function resetGeeLayerStoreForTest(): void {
  storeState = {
    layers: [],
    swipe: {
      enabled: false,
      positionPct: 50,
      leftLayerId: null,
      rightLayerId: null,
    },
    zoomTarget: null,
  };
  emitChange();
}

export function pushGeeMapLayer(
  input: Omit<GeeMapLayerItem, "visible" | "opacity" | "pinned"> & {
    visible?: boolean;
    opacity?: number;
    pinned?: boolean;
  },
  autoZoom = true
): GeeMapLayerItem {
  const nextLayer: GeeMapLayerItem = {
    ...input,
    visible: input.visible ?? true,
    opacity: input.opacity ?? 0.88,
    pinned: input.pinned ?? false,
  };
  const filtered = storeState.layers.filter((item) => item.id !== nextLayer.id);
  const layers = [nextLayer, ...filtered];
  storeState = {
    ...storeState,
    layers,
    zoomTarget: autoZoom
      ? { bbox: nextLayer.bbox, nonce: (storeState.zoomTarget?.nonce ?? 0) + 1 }
      : storeState.zoomTarget,
  };
  emitChange();
  return nextLayer;
}

export function pushGeeArtifactToMap(
  record: GeeArtifactRecord,
  mode: "single" | "swipe" = "single"
): void {
  if (
    mode === "swipe" &&
    (record.before_tile_url_template || record.before_preview_data_url) &&
    (record.after_tile_url_template || record.after_preview_data_url)
  ) {
    const beforeId = `${record.artifact_id}-before`;
    const afterId = `${record.artifact_id}-after`;
    const beforeLayer: GeeMapLayerItem = {
      id: beforeId,
      title: t("mapWorkbench.gee.beforePeriodSuffix", { title: record.title }),
      datasetId: record.dataset_id,
      operation: "before",
      bbox: record.bbox,
      visible: true,
      opacity: 0.92,
      pinned: true,
      tileUrlTemplate: record.before_tile_url_template,
      previewDataUrl: record.before_preview_data_url,
      visMin: -1,
      visMax: 1,
      palette: ["#d73027", "#fee08b", "#1a9850"],
      unit: record.unit,
    };
    const afterLayer: GeeMapLayerItem = {
      id: afterId,
      title: t("mapWorkbench.gee.afterPeriodSuffix", { title: record.title }),
      datasetId: record.dataset_id,
      operation: "after",
      bbox: record.bbox,
      visible: true,
      opacity: 0.92,
      pinned: true,
      tileUrlTemplate: record.after_tile_url_template,
      previewDataUrl: record.after_preview_data_url,
      visMin: -1,
      visMax: 1,
      palette: ["#d73027", "#fee08b", "#1a9850"],
      unit: record.unit,
    };
    const rest = storeState.layers.filter((l) => l.id !== beforeId && l.id !== afterId);
    storeState = {
      layers: [afterLayer, beforeLayer, ...rest],
      swipe: {
        enabled: true,
        positionPct: 50,
        leftLayerId: beforeId,
        rightLayerId: afterId,
      },
      zoomTarget: { bbox: record.bbox, nonce: (storeState.zoomTarget?.nonce ?? 0) + 1 },
    };
    emitChange();
    return;
  }

  pushGeeMapLayer(
    {
      id: record.artifact_id,
      title: record.title,
      datasetId: record.dataset_id,
      operation: record.operation,
      bbox: record.bbox,
      tileUrlTemplate: record.tile_url_template,
      previewDataUrl: record.preview_data_url,
      visMin: record.vis_min,
      visMax: record.vis_max,
      palette: Array.isArray(record.palette) ? record.palette : [],
      unit: record.unit || "",
      stats: record.stats,
      pinned: false,
    },
    true
  );
}

export function updateGeeMapLayer(
  id: string,
  patch: Partial<Pick<GeeMapLayerItem, "visible" | "opacity" | "pinned" | "title">>
): void {
  storeState = {
    ...storeState,
    layers: storeState.layers.map((layer) => (layer.id === id ? { ...layer, ...patch } : layer)),
  };
  emitChange();
}

export function removeGeeMapLayer(id: string): void {
  const nextLayers = storeState.layers.filter((layer) => layer.id !== id);
  const nextLeft = storeState.swipe.leftLayerId === id ? null : storeState.swipe.leftLayerId;
  const nextRight = storeState.swipe.rightLayerId === id ? null : storeState.swipe.rightLayerId;
  storeState = {
    ...storeState,
    layers: nextLayers,
    swipe: {
      ...storeState.swipe,
      leftLayerId: nextLeft,
      rightLayerId: nextRight,
      enabled: Boolean(storeState.swipe.enabled && nextLeft && nextRight),
    },
  };
  emitChange();
}

export function clearUnpinnedGeeMapLayers(): void {
  const kept = storeState.layers.filter((layer) => layer.pinned);
  const keptIds = new Set(kept.map((l) => l.id));
  const nextLeft =
    storeState.swipe.leftLayerId && keptIds.has(storeState.swipe.leftLayerId)
      ? storeState.swipe.leftLayerId
      : null;
  const nextRight =
    storeState.swipe.rightLayerId && keptIds.has(storeState.swipe.rightLayerId)
      ? storeState.swipe.rightLayerId
      : null;
  storeState = {
    ...storeState,
    layers: kept,
    swipe: {
      ...storeState.swipe,
      leftLayerId: nextLeft,
      rightLayerId: nextRight,
      enabled: Boolean(storeState.swipe.enabled && nextLeft && nextRight),
    },
  };
  emitChange();
}

export function setGeeSwipeState(patch: Partial<GeeSwipeState>): void {
  const merged: GeeSwipeState = {
    ...storeState.swipe,
    ...patch,
    positionPct:
      typeof patch.positionPct === "number"
        ? Math.max(2, Math.min(98, patch.positionPct))
        : storeState.swipe.positionPct,
  };
  if (merged.enabled && (!merged.leftLayerId || !merged.rightLayerId) && storeState.layers.length >= 2) {
    merged.leftLayerId = merged.leftLayerId ?? storeState.layers[1].id;
    merged.rightLayerId = merged.rightLayerId ?? storeState.layers[0].id;
  }
  storeState = {
    ...storeState,
    swipe: merged,
  };
  emitChange();
}

export function zoomToGeeBbox(bbox: [number, number, number, number]): void {
  storeState = {
    ...storeState,
    zoomTarget: { bbox, nonce: (storeState.zoomTarget?.nonce ?? 0) + 1 },
  };
  emitChange();
}
