import type { XyzTileEstimate } from "./xyzTileEstimator.ts";

export const WAYBACK_TILE_ORIGIN = "https://wayback-a.maptiles.arcgis.com";

export interface WaybackVersion {
  release_num: string;
  release_date: string;
  label: string;
  item_url: string;
  metadata_layer_url: string;
  layer_identifier: string;
  tile_url_template: string;
}

export interface WaybackCaptureEntry {
  capture_id: string;
  capture_date: string;
  release_num: string;
  release_date: string;
  tile_url_template: string;
  coverage_ratio: number;
  source_info: string;
  resolution_m: number | null;
  accuracy_m: number | null;
  geometry_hash: string;
  layer_id: number;
}

export interface WaybackReleaseSummary {
  release_num: string;
  release_date: string;
  label: string;
  tile_url_template: string;
  has_local_change: boolean;
  coverage_ratio: number;
  capture_count: number;
  capture_dates: string[];
  primary_source: string;
  min_resolution_m: number | null;
  from_dead_service: boolean;
}

export interface WaybackScanResult {
  scan_mode: string;
  zoom: number;
  bbox: [number, number, number, number];
  effective_layer: number;
  used_fallback_layer: boolean;
  total_releases_scanned: number;
  changed_releases_count: number;
  unique_captures_count: number;
  captures: WaybackCaptureEntry[];
  release_summaries: WaybackReleaseSummary[];
}

export interface WaybackVersionsResponse {
  ok: boolean;
  count: number;
  cache_source: string;
  fetched_at_epoch_secs: number;
  versions: WaybackVersion[];
}

export interface WaybackProbeZoomResult {
  ok: boolean;
  version_id: string;
  max_zoom: number;
  probe_point?: [number, number];
  no_imagery_found?: boolean;
}

export interface WaybackScanStatusResponse {
  ok: boolean;
  scan_id: string;
  status: "running" | "completed" | "cancelled" | "failed" | "not_found";
  scan_mode?: string;
  completed: number;
  total: number;
  effective_layer?: number;
  from_cache?: boolean;
  error?: string | null;
  result?: WaybackScanResult | null;
}

export interface WaybackPeriodSelection {
  releaseNum: string;
  dateLabel: string;
  label?: string;
}

export function isValidWaybackVersionId(raw: unknown): raw is string {
  const s = String(raw ?? "").trim();
  return /^\d{1,16}$/.test(s);
}

export function isValidWaybackSourceKey(raw: unknown): boolean {
  const s = String(raw ?? "").trim();
  return /^wayback_\d{1,16}$/i.test(s);
}

export function waybackTileUrlTemplate(releaseNum: string): string {
  const clean = String(releaseNum ?? "").trim();
  if (!isValidWaybackVersionId(clean)) {
    throw new Error(`INVALID_WAYBACK_VERSION_ID: ${clean}`);
  }
  return `${WAYBACK_TILE_ORIGIN}/arcgis/rest/services/world_imagery/mapserver/tile/${clean}/{z}/{y}/{x}`;
}

export function formatWaybackSubfolderName(
  releaseNum: string,
  releaseOrCaptureDate?: string | null,
): string {
  const cleanId = String(releaseNum ?? "").trim();
  if (!isValidWaybackVersionId(cleanId)) {
    throw new Error(`INVALID_WAYBACK_VERSION_ID: ${cleanId}`);
  }
  const safeDate = String(releaseOrCaptureDate ?? "")
    .trim()
    .replace(/[^A-Za-z0-9_-]/g, "")
    .slice(0, 16);
  return safeDate ? `wayback_${safeDate}_id${cleanId}` : `wayback_id${cleanId}`;
}

export function joinOutputSubdir(baseDir: string, subdir: string): string {
  const trimmedBase = String(baseDir ?? "").trim().replace(/[\\/]+$/, "");
  const cleanSub = String(subdir ?? "")
    .trim()
    .replace(/^[\\/]+|[\\/]+$/g, "");
  if (!trimmedBase) return cleanSub;
  if (!cleanSub) return trimmedBase;
  const sep = trimmedBase.includes("\\") ? "\\" : "/";
  return `${trimmedBase}${sep}${cleanSub}`;
}

export function scaleWaybackEstimateForPeriods(
  estimate: XyzTileEstimate,
  periodCount: number,
): {
  periodCount: number;
  totalTiles: number;
  totalNetworkDownloadBytes: number;
  totalEstimatedOutputBytes: number;
  exceedsHardLimit: boolean;
} {
  const n = Math.max(1, Math.round(periodCount));
  return {
    periodCount: n,
    totalTiles: estimate.totalTiles * n,
    totalNetworkDownloadBytes: estimate.totalNetworkDownloadBytes * n,
    totalEstimatedOutputBytes: estimate.totalEstimatedOutputBytes * n,
    exceedsHardLimit: estimate.exceedsHardLimit,
  };
}
