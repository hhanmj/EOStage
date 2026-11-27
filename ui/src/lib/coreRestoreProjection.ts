import type {
  DesktopCoreRestoreSnapshot,
  DesktopApplicationError,
  DesktopJsonObject,
  DesktopWorkbenchPageSnapshot,
} from "./desktopShellBridge.ts";
import type { Bbox } from "./bridge.ts";
import type { OrbitRecheckInfo } from "./generated/v3-contracts.ts";
import type { WrappedAoiEnvelope } from "./antimeridian.ts";
import { canonicalDemIdentityTimestamp } from "./demIdentityTimestamp.ts";
import { parseCanonicalGeometry, parseRenderGeometry } from "./geojsonGeometry.ts";
import { matchRenderCopyParity } from "./renderCopyParity.ts";

// The Store Workbench and normal Catalog presentation pages are capped at
// 250 items. This upper bound also permits the independent Golden map-overview
// projection.
export const MAX_UI_WORKBENCH_PAGE_SIZE = 250;
export const MAX_UI_TASK_PAGE_SIZE = 100;

const TASK_STATUSES = [
  "queued",
  "running",
  "paused",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
] as const;
const TASK_ITEM_STATUSES = [
  "queued",
  "active",
  "paused",
  "completed",
  "failed",
  "skipped",
  "cancelled",
  "interrupted",
] as const;

const DOWNLOAD_LAYOUTS = [
  "direct",
  "sentinel1_slc",
  "sentinel_orbit_aux_poeorb",
  "sar_dataset_processing",
] as const;

const LEGACY_COMPATIBILITY_STATUSES = ["adapted_read_only", "unavailable"] as const;
const LEGACY_REASON_CODES = [
  "sentinel1_slc_display_fields_adapted",
  "malformed_snapshot",
  "unsupported_persistence_version",
  "task_envelope_mismatch",
  "missing_or_invalid_runtime",
  "runtime_envelope_mismatch",
  "ambiguous_legacy_evidence",
] as const;

const MAX_CANONICAL_GEOMETRY_BYTES = 8 * 1024 * 1024;
const MAX_DISPLAY_GEOMETRY_BYTES = 17 * 1024 * 1024;
const MAX_DISPLAY_RENDER_PAYLOAD_BYTES = 17 * 1024 * 1024;
export const MAX_PROJECTED_RENDER_LONGITUDE = 900;
// Use a small, magnitude-scaled ULP window rather than a fixed decimal
// tolerance. The exact ±180 sign tie remains a separate bit-exact rule.
const ULP_FACTOR = 8;
const UTF8_ENCODER = new TextEncoder();

type JsonRecord = Record<string, unknown>;

function serializedJsonBytes(value: unknown): number | null {
  try {
    const serialized = JSON.stringify(value);
    return typeof serialized === "string" ? UTF8_ENCODER.encode(serialized).byteLength : null;
  } catch {
    return null;
  }
}

function ulpTolerance(left: number, right: number): number {
  const magnitude = Math.max(1, Math.abs(left), Math.abs(right));
  return Number.EPSILON * magnitude * ULP_FACTOR;
}

function nearlyEqual(left: number, right: number): boolean {
  return Math.abs(left - right) <= ulpTolerance(left, right);
}

type TaskStatus = (typeof TASK_STATUSES)[number];
type DownloadLayout = (typeof DOWNLOAD_LAYOUTS)[number];

export type CoreTaskOperationProjection = {
  operation_id: string;
  kind: string;
  phase: string;
  region_id: string;
  approved_worker_operation: string;
  source_provider?: string | null;
  input_path?: string | null;
  source_vertical_datum?: string | null;
  convert?: boolean | null;
};

export type CoreTaskActiveItemProjection = {
  item_id: string;
  bytes_completed: number;
  total_bytes: number | null;
};

/** Active items by what their current progress row reports. */
export type CoreTaskActiveProgressProjection = {
  without_progress: number;
  transferring: number;
  idle: number;
  /** Items waiting for a local resource (the NISAR output writer); absent on hosts before downloader D1. */
  waiting?: number;
};

/** A distinct stable failure code of the task's failed items. */
export type CoreTaskFailureCodeProjection = {
  code: string;
  local: boolean;
  /** Every such failure could not verify the server certificate (L5 recheck 2026-10-19); absent when false. */
  tls_certificate?: boolean;
  /** Every such failure declares a network transport cause (downloader D1d); absent when false. */
  network?: boolean;
};

export type CoreTaskSummaryProjection = {
  task_id: string;
  batch_id: string;
  provider_id: string;
  dataset_id: string | null;
  concurrency?: number | null;
  task_type?: string | null;
  created_at: string;
  status: TaskStatus;
  revision: number;
  updated_at: string;
  selected_count: number;
  delivery_requested_count?: number | null;
  delivery_rejected_count?: number | null;
  checked_count: number;
  active_count: number;
  paused_count: number;
  queued_count: number;
  completed_count: number;
  failed_count: number;
  skipped_count: number;
  cancelled_count: number;
  terminal_bytes: number;
  active_bytes: number;
  other_bytes: number;
  transferred_bytes: number;
  total_bytes: number | null;
  source_total_bytes?: number | null;
  bytes_per_second: number;
  transfer_active_millis: number;
  network_bytes_received?: number | null;
  reusable_verified_bytes?: number | null;
  reused_verified_bytes?: number | null;
  active_item: CoreTaskActiveItemProjection | null;
  /** Absent on hosts that predate the phase facts. */
  active_progress?: CoreTaskActiveProgressProjection | null;
  /** Absent on hosts that predate the failure facts. */
  failure_codes?: CoreTaskFailureCodeProjection[] | null;
  /** Downloader D3-2: the local pre-scan and its reuse; absent on older hosts. */
  local_scan?: CoreTaskLocalScanProjection | null;
  output_root: string;
  layout: DownloadLayout;
  operation: CoreTaskOperationProjection | null;
  source_task_id?: string | null;
  auxiliary_kind?: string | null;
  actionable: boolean;
};

export const LOCAL_SCAN_STATES = ["none", "pending", "scanning", "done", "stopped"] as const;

/** Downloader D3-2: where a task's local pre-scan stands and what it reused. */
export type CoreTaskLocalScanProjection = {
  state: (typeof LOCAL_SCAN_STATES)[number];
  scanned_items: number;
  total_items: number;
  reused_count: number;
  unverified_count: number;
};

export type CoreLegacyTaskWarningProjection = {
  task_id: string;
  provider_id: string;
  created_at: string;
  status: TaskStatus | null;
  reason_code: (typeof LEGACY_REASON_CODES)[number];
  compatibility_status: (typeof LEGACY_COMPATIBILITY_STATUSES)[number];
  /** Compatibility warnings are display-only and never actionable. */
  actionable: false;
};

export type CoreWorkbenchSourceAttemptProjection = {
  provider_id: string;
  native_collection_id: string;
  outcome: "fallback" | "succeeded";
  fallback_reason: "transient_network" | "rate_limited" | "server_error" | null;
};

export type CoreWorkbenchSummaryProjection = {
  batch_id: string;
  provider_id: string;
  created_at: string;
  aoi_name: string;
  query: {
    provider: string | null;
    dataset: string | null;
    start: string | null;
    end: string | null;
    target_count: number | null;
  };
  search_stats: {
    matched_count: number | null;
    provider_returned_count: number;
    frozen_candidate_count: number;
    truncated: boolean;
    source_attempts: CoreWorkbenchSourceAttemptProjection[];
    source: "ASF" | "ASF_BATCH" | "ASF_CMR" | "TYPED_CATALOG";
  } | null;
  candidate_count: number;
  checked_count: number;
  current_page_item_count: number;
  page_index: number;
  page_size: number;
  revision: number;
  deferred: boolean;
  reason_code: "candidate_payload_deferred" | null;
};

export type CoreDemSourceIdentityProjection = {
  dataset: string;
  provider: string;
  updated_at: string;
};

export type CoreRestoreProjection = {
  readiness: {
    schema_version: number;
    ready: boolean;
  };
  workspace_context: JsonRecord | null;
  region_aoi: JsonRecord | null;
  region_aois: JsonRecord[];
  dem_source_identity: CoreDemSourceIdentityProjection | null;
  tasks: CoreTaskSummaryProjection[];
  legacy_task_warnings: CoreLegacyTaskWarningProjection[];
  workbench: CoreWorkbenchSummaryProjection | null;
};

export type WorkbenchPageItemProjection = {
  ordinal: number;
  item: {
    item_id: string;
    provider_id: string;
    display_name: string;
    acquisition_datetime: string | null;
    footprint_geojson: JsonRecord | null;
    metadata: JsonRecord;
    payload: JsonRecord;
    asset_access: JsonRecord | null;
    execution_policy: JsonRecord | null;
  };
  checked: boolean;
};

export type WorkbenchPageProjection = {
  batch_id: string;
  revision: number;
  page_index: number;
  page_size: number;
  total_count: number;
  checked_count: number;
  items: WorkbenchPageItemProjection[];
  render: WorkbenchPageRenderAuthorityProjection | null;
};

export type DisplayBboxProjection = {
  west: number;
  south: number;
  east: number;
  north: number;
};

export type WorkbenchPageRenderItemProjection = {
  item_id: string;
  render_geometry: JsonRecord | null;
  render_bbox: DisplayBboxProjection | null;
};

export type WorkbenchPageRenderAuthorityProjection = {
  region_id: string;
  revision: number;
  items: WorkbenchPageRenderItemProjection[];
};

const LOCAL_REUSE_DISPOSITIONS = ["reused", "renamed_conflict", "reacquire_requested"] as const;
const LOCAL_REUSE_GRADES = ["service_checksum", "ledger_identity", "ledger_rehash", "size_and_format"] as const;

/** Downloader D3: how a task item met an existing file at its planned destination. */
export type TaskItemLocalReuseProjection = {
  disposition: (typeof LOCAL_REUSE_DISPOSITIONS)[number];
  grade: (typeof LOCAL_REUSE_GRADES)[number] | null;
  conflict_reason: string | null;
  conflict_file_name: string | null;
};

export type TaskPageItemProjection = WorkbenchPageItemProjection & {
  status: (typeof TASK_ITEM_STATUSES)[number];
  bytes_completed: number;
  total_bytes: number | null;
  source_total_bytes: number | null;
  bytes_per_second: number;
  attempt: number;
  destination: string | null;
  failure: {
    code: string;
    reason_zh: string;
    retryable: boolean;
  } | null;
  finished_at: string | null;
  retryable: boolean;
  orbit_recheck: OrbitRecheckInfo | null;
  /** Absent (null) on hosts before downloader D3 and for items without a decision. */
  local_reuse: TaskItemLocalReuseProjection | null;
};

export type TaskPageProjection = {
  task_id: string;
  revision: number;
  page_index: number;
  page_size: number;
  total_count: number;
  source_total_bytes: number | null;
  network_bytes_received: number | null;
  items: TaskPageItemProjection[];
};

export type CoreRestoreProjectionResult =
  | { ok: true; core: CoreRestoreProjection; aoi_display: CoreAoiDisplayAuthorityProjection | null }
  | DesktopApplicationError
  | { ok: false; error: string; code: "V3_APPLICATION_INVALID_SNAPSHOT" };

export type WorkbenchPageProjectionResult =
  | { ok: true; page: WorkbenchPageProjection; render: WorkbenchPageRenderAuthorityProjection | null }
  | DesktopApplicationError
  | { ok: false; error: string; code: "V3_APPLICATION_INVALID_SNAPSHOT" };

export type CoreAoiDisplayAuthorityProjection = {
  region_id: string;
  revision: number;
  envelope: WrappedAoiEnvelope;
  render_geometry: JsonRecord;
  render_bbox: DisplayBboxProjection;
};

function record(value: unknown): JsonRecord | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord
    : null;
}

function string(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function optionalString(value: unknown): string | null | undefined {
  if (value === null) return null;
  return typeof value === "string" ? value : undefined;
}

function safeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function boolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function nullableRecord(value: unknown): JsonRecord | null | undefined {
  if (value === null) return null;
  return record(value) ?? undefined;
}

function exactKeys(value: JsonRecord, keys: readonly string[]): boolean {
  const allowed = new Set(keys);
  return Object.keys(value).every((key) => allowed.has(key));
}

function enumValue<T extends readonly string[]>(value: unknown, values: T): T[number] | null {
  return typeof value === "string" && (values as readonly string[]).includes(value)
    ? value as T[number]
    : null;
}

function parseTaskOperation(value: unknown): CoreTaskOperationProjection | null | undefined {
  if (value === null) return null;
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, [
      "operation_id", "kind", "phase", "region_id", "approved_worker_operation", "source_provider",
      "input_path", "source_vertical_datum", "convert",
    ])
  ) return undefined;
  const operationId = string(source.operation_id);
  const kind = string(source.kind);
  const phase = string(source.phase);
  const regionId = string(source.region_id);
  const approvedWorkerOperation = string(source.approved_worker_operation);
  const sourceProvider = source.source_provider === undefined || source.source_provider === null
    ? null
    : string(source.source_provider);
  const inputPath = source.input_path === undefined || source.input_path === null
    ? null
    : string(source.input_path);
  const sourceVerticalDatum = source.source_vertical_datum === undefined || source.source_vertical_datum === null
    ? null
    : string(source.source_vertical_datum);
  const convert = source.convert === undefined || source.convert === null
    ? null
    : boolean(source.convert);
  if (!operationId || !kind || !phase || !regionId || !approvedWorkerOperation
      || (source.source_provider !== undefined && source.source_provider !== null && !sourceProvider)
      || (source.input_path !== undefined && source.input_path !== null && !inputPath)
      || (source.source_vertical_datum !== undefined && source.source_vertical_datum !== null && !sourceVerticalDatum)
      || (source.convert !== undefined && source.convert !== null && convert === null)) return undefined;
  return {
    operation_id: operationId,
    kind,
    phase,
    region_id: regionId,
    approved_worker_operation: approvedWorkerOperation,
    ...(sourceProvider ? { source_provider: sourceProvider } : {}),
    ...(inputPath ? { input_path: inputPath } : {}),
    ...(sourceVerticalDatum ? { source_vertical_datum: sourceVerticalDatum } : {}),
    ...(convert !== null ? { convert } : {}),
  };
}

function canonicalIdentityCode(value: unknown): value is string {
  return typeof value === "string" && value.length >= 1 && value.length <= 128 &&
    value === value.trim() && value === value.toUpperCase() && /^[A-Z0-9_.-]+$/.test(value);
}

function parseDemSourceIdentity(value: unknown): CoreDemSourceIdentityProjection | null | undefined {
  if (value === null || value === undefined) return null;
  const source = record(value);
  const updatedAt = canonicalDemIdentityTimestamp(source?.updated_at);
  if (!source || !exactKeys(source, ["dataset", "provider", "updated_at"]) ||
      Object.keys(source).length !== 3 || !canonicalIdentityCode(source.dataset) ||
      !canonicalIdentityCode(source.provider) ||
      updatedAt === null || updatedAt !== source.updated_at) {
    return undefined;
  }
  return {
    dataset: source.dataset,
    provider: source.provider,
    updated_at: updatedAt,
  };
}

function parseAoiDisplayBbox(value: unknown): Bbox | null {
  const source = record(value);
  if (!source || !exactKeys(source, ["west", "east", "south", "north", "crs"])) return null;
  const west = typeof source.west === "number" && Number.isFinite(source.west) ? source.west : null;
  const east = typeof source.east === "number" && Number.isFinite(source.east) ? source.east : null;
  const south = typeof source.south === "number" && Number.isFinite(source.south) ? source.south : null;
  const north = typeof source.north === "number" && Number.isFinite(source.north) ? source.north : null;
  if (
    west === null || east === null || south === null || north === null ||
    source.crs !== "EPSG:4326" || west < -180 || east > 180 || west >= east ||
    south < -90 || north > 90 || south >= north
  ) return null;
  return { west, east, south, north, crs: "EPSG:4326" };
}

function parseDisplayBbox(value: unknown): DisplayBboxProjection | null {
  const source = record(value);
  if (!source || !exactKeys(source, ["west", "south", "east", "north"])) return null;
  const west = typeof source.west === "number" && Number.isFinite(source.west) ? source.west : null;
  const east = typeof source.east === "number" && Number.isFinite(source.east) ? source.east : null;
  const south = typeof source.south === "number" && Number.isFinite(source.south) ? source.south : null;
  const north = typeof source.north === "number" && Number.isFinite(source.north) ? source.north : null;
  if (
    west === null || east === null || south === null || north === null ||
    west < -MAX_PROJECTED_RENDER_LONGITUDE || east > MAX_PROJECTED_RENDER_LONGITUDE ||
    west >= east || south < -90 || north > 90 || south >= north
  ) return null;
  return { west, south, east, north };
}

/** Strict bounded parser for native render copies. */
function parseDisplayGeometry(value: unknown, maximumBytes = MAX_DISPLAY_GEOMETRY_BYTES): JsonRecord | null {
  return parseRenderGeometry(value, maximumBytes, MAX_PROJECTED_RENDER_LONGITUDE);
}

type DisplayPosition = number[];
type DisplayPolygon = DisplayPosition[][];

function displayGeometryPolygons(geometry: JsonRecord): DisplayPolygon[] | null {
  if (geometry.type === "Polygon" && Array.isArray(geometry.coordinates)) {
    return [geometry.coordinates as DisplayPolygon];
  }
  if (geometry.type === "MultiPolygon" && Array.isArray(geometry.coordinates)) {
    return geometry.coordinates as DisplayPolygon[];
  }
  return null;
}

function displayGeometryBounds(geometry: JsonRecord): DisplayBboxProjection | null {
  const stack: unknown[] = [geometry.coordinates];
  let west = Infinity;
  let east = -Infinity;
  let south = Infinity;
  let north = -Infinity;
  while (stack.length) {
    const value = stack.pop();
    if (Array.isArray(value) && value.length >= 2 &&
        typeof value[0] === "number" && typeof value[1] === "number") {
      west = Math.min(west, value[0]);
      east = Math.max(east, value[0]);
      south = Math.min(south, value[1]);
      north = Math.max(north, value[1]);
    } else if (Array.isArray(value)) {
      for (const item of value) stack.push(item);
    }
  }
  if (!Number.isFinite(west) || !Number.isFinite(east) || !Number.isFinite(south) || !Number.isFinite(north)) return null;
  return {
    west,
    east,
    south,
    north,
  };
}

function displayGeometryWithinBbox(
  geometry: JsonRecord,
  bbox: DisplayBboxProjection,
  exact = true,
): boolean {
  const bounds = displayGeometryBounds(geometry);
  if (!bounds) return false;
  return exact
    ? bounds.west === bbox.west && bounds.east === bbox.east &&
      bounds.south === bbox.south && bounds.north === bbox.north
    : bounds.west >= bbox.west && bounds.east <= bbox.east &&
      bounds.south >= bbox.south && bounds.north <= bbox.north;
}

function displayGeometryCopiesRelevant(
  geometry: JsonRecord,
  envelope: WrappedAoiEnvelope,
  requireEveryComponent = true,
): boolean {
  const polygons = displayGeometryPolygons(geometry);
  if (!polygons) return false;
  let relevantComponent = false;
  for (const polygon of polygons) {
    const bounds = displayGeometryBounds({ type: "MultiPolygon", coordinates: [polygon] });
    if (!bounds || bounds.east - bounds.west > 360 + ulpTolerance(bounds.east, bounds.west)) return false;
    const longitudeOverlap = Math.min(bounds.east, envelope.displayEast) - Math.max(bounds.west, envelope.displayWest);
    const latitudeOverlap = Math.min(bounds.north, envelope.north) - Math.max(bounds.south, envelope.south);
    // Every emitted render component must have a positive longitude overlap;
    // latitude is scene-level so a far-latitude component may accompany one
    // component that actually intersects the AOI.
    if (!(longitudeOverlap > 0)) return false;
    if (latitudeOverlap > 0) relevantComponent = true;
    else if (requireEveryComponent) return false;
  }
  return relevantComponent;
}

function canonicalGeometryIsGeographic(geometry: JsonRecord): boolean {
  const polygons = displayGeometryPolygons(geometry);
  return polygons !== null && polygons.every((polygon) => polygon.every((ring) => ring.every((position) =>
    position[0]! >= -180 && position[0]! <= 180 && position[1]! >= -90 && position[1]! <= 90,
  )));
}

function geometryHasFullWorldEdge(geometry: JsonRecord): boolean {
  const polygons = displayGeometryPolygons(geometry);
  if (!polygons) return false;
  for (const polygon of polygons) {
    for (const ring of polygon) {
      for (let index = 1; index < ring.length; index += 1) {
        const previous = ring[index - 1]!;
        const current = ring[index]!;
        if (
          previous[0] === -180 && current[0] === 180 ||
          previous[0] === 180 && current[0] === -180
        ) return true;
      }
    }
  }
  return false;
}

function geometryWorldCopyOfCanonical(render: JsonRecord, canonical: unknown, requireEveryCanonical = true): boolean {
  const canonicalGeometry = parseCanonicalGeometry(canonical, MAX_CANONICAL_GEOMETRY_BYTES);
  const renderPolygons = displayGeometryPolygons(render);
  const canonicalPolygons = canonicalGeometry
    ? displayGeometryPolygons({ type: canonicalGeometry.type, coordinates: canonicalGeometry.coordinates })
    : null;
  if (!canonicalGeometry || !canonicalGeometryIsGeographic(canonicalGeometry) || !canonicalPolygons || !renderPolygons) {
    return false;
  }
  return matchRenderCopyParity(canonicalPolygons, renderPolygons, requireEveryCanonical);
}

export function parseCoreAoiDisplay(
  value: unknown,
  canonicalGeometry?: unknown,
): CoreAoiDisplayAuthorityProjection | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  const envelope = source && record(source.envelope);
  const renderGeometry = source && parseDisplayGeometry(source.render_geometry);
  const renderBbox = source && parseDisplayBbox(source.render_bbox);
  if (
    !source ||
    !envelope ||
    !renderGeometry ||
    !renderBbox ||
    // The native AOI authority is derived from this exact render geometry;
    // accepting a containing bbox would allow an envelope to drift beyond
    // the geometry that the map actually draws.
    !displayGeometryWithinBbox(renderGeometry, renderBbox) ||
    canonicalGeometry !== undefined && !geometryWorldCopyOfCanonical(renderGeometry, canonicalGeometry) ||
    !exactKeys(source, ["region_id", "revision", "envelope", "render_geometry", "render_bbox"]) ||
    !exactKeys(envelope, ["south", "north", "crosses_antimeridian", "display_west", "display_east", "request_windows"])
  ) return undefined;
  const regionId = string(source.region_id);
  const revision = safeInteger(source.revision);
  const south = typeof envelope.south === "number" && Number.isFinite(envelope.south) ? envelope.south : null;
  const north = typeof envelope.north === "number" && Number.isFinite(envelope.north) ? envelope.north : null;
  const displayWest = typeof envelope.display_west === "number" && Number.isFinite(envelope.display_west)
    ? envelope.display_west
    : null;
  const displayEast = typeof envelope.display_east === "number" && Number.isFinite(envelope.display_east)
    ? envelope.display_east
    : null;
  const crosses = boolean(envelope.crosses_antimeridian);
  const windows = Array.isArray(envelope.request_windows)
    ? envelope.request_windows.map(parseAoiDisplayBbox)
    : null;
  if (
    !regionId || revision === null || south === null || north === null || displayWest === null || displayEast === null ||
    crosses === null || !windows || windows.length < 1 || windows.length > 2 || windows.some((window) => window === null) ||
    south < -90 || north > 90 || south >= north || displayWest < -180 || displayWest >= 180 ||
    displayEast <= displayWest || displayEast - displayWest > 360 ||
    windows.some((window) => window!.south !== south || window!.north !== north) ||
    (crosses ? windows.length !== 2 : windows.length !== 1)
  ) return undefined;
  const parsedWindows = windows as Bbox[];
  const windowsMatchDisplay = crosses
    ? parsedWindows[0]!.west === displayWest
      && parsedWindows[0]!.east === 180
      && parsedWindows[1]!.west === -180
      && parsedWindows[1]!.east + 360 === displayEast
    : parsedWindows[0]!.west === displayWest
      && parsedWindows[0]!.east === displayEast;
  if (!windowsMatchDisplay) return undefined;
  const parsedEnvelope: WrappedAoiEnvelope = {
    south,
    north,
    crossesAntimeridian: crosses,
    displayWest,
    displayEast,
    requestWindows: parsedWindows,
  };
  const explicitFullWorld = displayWest === -180 && displayEast === 180;
  if (explicitFullWorld) {
    const fullWorldCanonical = canonicalGeometry === undefined
      ? renderGeometry
      : parseCanonicalGeometry(canonicalGeometry, MAX_CANONICAL_GEOMETRY_BYTES);
    // A bbox spanning [-180,180] is not sufficient evidence of a global AOI:
    // two disconnected seam rectangles can have the same extrema. Require
    // the producer's exact raw full-world edge, matching Rust's authority.
    if (!fullWorldCanonical || !geometryHasFullWorldEdge(fullWorldCanonical)) return undefined;
  }
  if (
    renderBbox.south !== south || renderBbox.north !== north ||
    renderBbox.west !== displayWest || renderBbox.east !== displayEast ||
    renderBbox.east - renderBbox.west > 360 + ulpTolerance(renderBbox.east, renderBbox.west) &&
      !(displayWest === -180 && displayEast === 180) ||
    !displayGeometryCopiesRelevant(renderGeometry, parsedEnvelope)
  ) return undefined;
  return {
    region_id: regionId,
    revision,
    envelope: parsedEnvelope,
    render_geometry: renderGeometry,
    render_bbox: renderBbox,
  };
}

function parseTaskActiveItem(value: unknown): CoreTaskActiveItemProjection | null | undefined {
  if (value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["item_id", "bytes_completed", "total_bytes"])) return undefined;
  const itemId = string(source.item_id);
  const bytesCompleted = safeInteger(source.bytes_completed);
  const totalBytes = source.total_bytes === null ? null : safeInteger(source.total_bytes);
  if (!itemId || bytesCompleted === null || totalBytes === undefined) return undefined;
  return { item_id: itemId, bytes_completed: bytesCompleted, total_bytes: totalBytes };
}

function parseTaskActiveProgress(value: unknown): CoreTaskActiveProgressProjection | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["without_progress", "transferring", "idle", "waiting"])) return undefined;
  const withoutProgress = safeInteger(source.without_progress);
  const transferring = safeInteger(source.transferring);
  const idle = safeInteger(source.idle);
  if (withoutProgress === null || transferring === null || idle === null) return undefined;
  if (source.waiting === undefined) return { without_progress: withoutProgress, transferring, idle };
  const waiting = safeInteger(source.waiting);
  if (waiting === null) return undefined;
  return { without_progress: withoutProgress, transferring, idle, waiting };
}

/** The Store sends at most eight entries (an overflow sentinel included). */
const MAX_TASK_FAILURE_CODES = 8;

function parseTaskFailureCodes(value: unknown): CoreTaskFailureCodeProjection[] | null | undefined {
  if (value === undefined || value === null) return null;
  if (!Array.isArray(value) || value.length > MAX_TASK_FAILURE_CODES) return undefined;
  const parsed: CoreTaskFailureCodeProjection[] = [];
  for (const entry of value) {
    const source = record(entry);
    if (!source || !exactKeys(source, ["code", "local", "tls_certificate", "network"])) return undefined;
    const code = string(source.code);
    const local = boolean(source.local);
    if (!code || !/^[A-Z0-9_]{1,128}$/.test(code) || local === null) return undefined;
    const failure: CoreTaskFailureCodeProjection = { code, local };
    for (const flag of ["tls_certificate", "network"] as const) {
      if (source[flag] === undefined) continue;
      const value = boolean(source[flag]);
      if (value === null) return undefined;
      failure[flag] = value;
    }
    parsed.push(failure);
  }
  return parsed;
}

/** null when absent; undefined when present but malformed. */
function parseTaskLocalScan(value: unknown): CoreTaskLocalScanProjection | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["state", "scanned_items", "total_items", "reused_count", "unverified_count"])) {
    return undefined;
  }
  const state = enumValue(source.state, LOCAL_SCAN_STATES);
  const scanned = safeInteger(source.scanned_items);
  const total = safeInteger(source.total_items);
  const reused = safeInteger(source.reused_count);
  const unverified = safeInteger(source.unverified_count);
  if (
    !state || scanned === null || total === null || reused === null || unverified === null ||
    scanned > total || unverified > reused
  ) return undefined;
  return { state, scanned_items: scanned, total_items: total, reused_count: reused, unverified_count: unverified };
}

export function parseCoreTaskSummary(value: unknown): CoreTaskSummaryProjection | null {
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, [
      "task_id", "batch_id", "provider_id", "dataset_id", "concurrency", "task_type", "created_at", "status", "revision", "updated_at",
      "selected_count", "delivery_requested_count", "delivery_rejected_count", "checked_count", "active_count", "paused_count", "queued_count",
      "completed_count", "failed_count", "skipped_count", "cancelled_count", "output_root",
      "terminal_bytes", "active_bytes", "other_bytes", "transferred_bytes", "total_bytes", "source_total_bytes", "bytes_per_second", "transfer_active_millis", "network_bytes_received", "reusable_verified_bytes", "reused_verified_bytes", "active_item", "active_progress", "failure_codes", "local_scan", "layout", "operation", "source_task_id", "auxiliary_kind", "actionable",
    ])
  ) return null;

  const taskId = string(source.task_id);
  const batchId = string(source.batch_id);
  const providerId = string(source.provider_id);
  const datasetId = source.dataset_id === undefined ? null : optionalString(source.dataset_id);
  const concurrency = source.concurrency === undefined || source.concurrency === null
    ? null
    : safeInteger(source.concurrency);
  const taskType = source.task_type === undefined || source.task_type === null
    ? null
    : string(source.task_type);
  const datasetIdValid = datasetId === null || (
    typeof datasetId === "string" && datasetId.trim() === datasetId &&
    Array.from(datasetId).length <= 128 && !/[\u0000-\u001f\u007f]/u.test(datasetId)
  );
  const createdAt = string(source.created_at);
  const status = enumValue(source.status, TASK_STATUSES);
  const revision = safeInteger(source.revision);
  const updatedAt = string(source.updated_at);
  const outputRoot = string(source.output_root);
  const layout = enumValue(source.layout, DOWNLOAD_LAYOUTS);
  const actionable = boolean(source.actionable);
  const operation = parseTaskOperation(source.operation);
  const sourceTaskId = source.source_task_id == null ? null : string(source.source_task_id);
  const auxiliaryKind = source.auxiliary_kind == null ? null : string(source.auxiliary_kind);
  const terminalBytes = safeInteger(source.terminal_bytes);
  const activeBytes = safeInteger(source.active_bytes);
  const otherBytes = safeInteger(source.other_bytes);
  const transferredBytes = safeInteger(source.transferred_bytes);
  const totalBytes = source.total_bytes === null ? null : safeInteger(source.total_bytes);
  const sourceTotalBytes = source.source_total_bytes == null
    ? null : safeInteger(source.source_total_bytes);
  const bytesPerSecond = typeof source.bytes_per_second === "number" && Number.isFinite(source.bytes_per_second)
    && source.bytes_per_second >= 0
    ? source.bytes_per_second
    : null;
  const transferActiveMillis = safeInteger(source.transfer_active_millis);
  const networkBytesReceived = source.network_bytes_received == null
    ? null : safeInteger(source.network_bytes_received);
  const reusableVerifiedBytes = source.reusable_verified_bytes == null
    ? null : safeInteger(source.reusable_verified_bytes);
  const reusedVerifiedBytes = source.reused_verified_bytes == null
    ? null : safeInteger(source.reused_verified_bytes);
  const activeItem = parseTaskActiveItem(source.active_item);
  const activeProgress = parseTaskActiveProgress(source.active_progress);
  const failureCodes = parseTaskFailureCodes(source.failure_codes);
  const localScan = parseTaskLocalScan(source.local_scan);
  const deliveryRequestedCount = source.delivery_requested_count == null
    ? null
    : safeInteger(source.delivery_requested_count);
  const deliveryRejectedCount = source.delivery_rejected_count == null
    ? null
    : safeInteger(source.delivery_rejected_count);
  const expectedTransferred = terminalBytes !== null && activeBytes !== null && otherBytes !== null
    ? terminalBytes + activeBytes + otherBytes
    : null;
  const counts = [
    source.selected_count,
    source.checked_count,
    source.active_count,
    source.paused_count,
    source.queued_count,
    source.completed_count,
    source.failed_count,
    source.skipped_count,
    source.cancelled_count,
  ].map(safeInteger);
  if (
    !taskId || !batchId || !providerId || datasetId === undefined || !datasetIdValid ||
    concurrency !== null && concurrency < 1 || source.concurrency !== undefined && source.concurrency !== null && concurrency === null ||
    source.task_type !== undefined && source.task_type !== null && taskType === null ||
    !createdAt || !status || revision === null || !updatedAt ||
    !outputRoot || !layout || actionable === null || operation === undefined ||
    source.source_task_id != null && sourceTaskId === null ||
    source.auxiliary_kind != null && auxiliaryKind === null ||
    terminalBytes === null ||
    activeBytes === null || otherBytes === null || transferredBytes === null || expectedTransferred === null ||
    !Number.isSafeInteger(expectedTransferred) || transferredBytes !== expectedTransferred ||
    totalBytes === undefined || bytesPerSecond === null || transferActiveMillis === null ||
    source.source_total_bytes != null && sourceTotalBytes === null ||
    source.network_bytes_received != null && networkBytesReceived === null ||
    source.reusable_verified_bytes != null && reusableVerifiedBytes === null ||
    source.reused_verified_bytes != null && reusedVerifiedBytes === null ||
    source.delivery_requested_count != null && deliveryRequestedCount === null ||
    source.delivery_rejected_count != null && deliveryRejectedCount === null ||
    (deliveryRequestedCount === null) !== (deliveryRejectedCount === null) ||
    deliveryRequestedCount !== null && deliveryRejectedCount !== null
      && (((providerId === "microsoft.planetary-computer" || providerId === "element84.earth-search")
        && (datasetId === "optical.landsat.c2.l2"
            && (batchId === `landsat-assets-batch-${taskId}` || batchId === `landsat-assets-${taskId}`)
          || datasetId === "optical.sentinel-2.l2a"
            && (batchId === `sentinel2-bands-batch-${taskId}` || batchId === `sentinel2-bands-${taskId}`)
          || (datasetId === "optical.hls.l30.v2" || datasetId === "optical.hls.s30.v2")
            && batchId === `hls-bands-batch-${taskId}`))
        || (providerId === "copernicus.dataspace.odata"
          && datasetId === "optical.sentinel-2.l1c"
          && batchId === `sentinel2-l1c-bands-batch-${taskId}`)
        // File-batch admission counts scenes, whereas selected_count counts assets.
        ? deliveryRequestedCount < 1 || deliveryRequestedCount > 250
          || deliveryRequestedCount <= deliveryRejectedCount || counts[0] === null
          || deliveryRequestedCount - deliveryRejectedCount > counts[0]
        : deliveryRequestedCount < 1 || deliveryRequestedCount !== counts[0]! + deliveryRejectedCount) ||
    activeItem === undefined || activeProgress === undefined || failureCodes === undefined ||
    localScan === undefined ||
    counts.some((item) => item === null)
  ) return null;

  return {
    task_id: taskId,
    batch_id: batchId,
    provider_id: providerId,
    dataset_id: datasetId,
    concurrency,
    task_type: taskType,
    created_at: createdAt,
    status,
    revision,
    updated_at: updatedAt,
    selected_count: counts[0]!,
    delivery_requested_count: deliveryRequestedCount,
    delivery_rejected_count: deliveryRejectedCount,
    checked_count: counts[1]!,
    active_count: counts[2]!,
    paused_count: counts[3]!,
    queued_count: counts[4]!,
    completed_count: counts[5]!,
    failed_count: counts[6]!,
    skipped_count: counts[7]!,
    cancelled_count: counts[8]!,
    terminal_bytes: terminalBytes,
    active_bytes: activeBytes,
    other_bytes: otherBytes,
    transferred_bytes: transferredBytes,
    total_bytes: totalBytes,
    source_total_bytes: sourceTotalBytes,
    bytes_per_second: bytesPerSecond,
    transfer_active_millis: transferActiveMillis,
    network_bytes_received: networkBytesReceived,
    reusable_verified_bytes: reusableVerifiedBytes,
    reused_verified_bytes: reusedVerifiedBytes,
    active_item: activeItem,
    active_progress: activeProgress,
    failure_codes: failureCodes,
    local_scan: localScan,
    output_root: outputRoot,
    layout,
    operation,
    ...(source.source_task_id !== undefined ? { source_task_id: sourceTaskId } : {}),
    ...(source.auxiliary_kind !== undefined ? { auxiliary_kind: auxiliaryKind } : {}),
    actionable,
  };
}

function parseWarning(value: unknown): CoreLegacyTaskWarningProjection | null {
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, ["task_id", "provider_id", "created_at", "status", "reason_code", "compatibility_status"])
  ) return null;
  const taskId = string(source.task_id);
  const providerId = string(source.provider_id);
  const createdAt = string(source.created_at);
  const status = source.status === null ? null : enumValue(source.status, TASK_STATUSES);
  const reasonCode = enumValue(source.reason_code, LEGACY_REASON_CODES);
  const compatibilityStatus = enumValue(source.compatibility_status, LEGACY_COMPATIBILITY_STATUSES);
  if (!taskId || !providerId || !createdAt || status === null && source.status !== null || !reasonCode || !compatibilityStatus) {
    return null;
  }
  return {
    task_id: taskId,
    provider_id: providerId,
    created_at: createdAt,
    status,
    reason_code: reasonCode,
    compatibility_status: compatibilityStatus,
    actionable: false,
  };
}

function parseWorkbenchSourceAttempt(value: unknown): CoreWorkbenchSourceAttemptProjection | null {
  const source = record(value);
  if (!source || !exactKeys(source, ["provider_id", "native_collection_id", "outcome", "fallback_reason"])) return null;
  const providerId = string(source.provider_id);
  const nativeCollectionId = string(source.native_collection_id);
  const outcome = source.outcome === "fallback" || source.outcome === "succeeded" ? source.outcome : null;
  const fallbackReason = source.fallback_reason === null
    ? null
    : source.fallback_reason === "transient_network" || source.fallback_reason === "rate_limited" || source.fallback_reason === "server_error"
      ? source.fallback_reason
      : undefined;
  if (
    !providerId || providerId.length > 128 || !/^[A-Za-z0-9._:-]+$/.test(providerId)
    || !nativeCollectionId || nativeCollectionId.length > 256 || !/^[A-Za-z0-9._:-]+$/.test(nativeCollectionId)
    || !outcome || fallbackReason === undefined
    || outcome === "fallback" && fallbackReason === null
    || outcome === "succeeded" && fallbackReason !== null
  ) return null;
  return {
    provider_id: providerId,
    native_collection_id: nativeCollectionId,
    outcome,
    fallback_reason: fallbackReason,
  };
}

function parseWorkbenchSummary(value: unknown): CoreWorkbenchSummaryProjection | null {
  if (value === null) return null;
  const source = record(value);
  const query = source && record(source.query);
  if (
    !source ||
    !query ||
    !exactKeys(source, [
      "batch_id", "provider_id", "created_at", "aoi_name", "query", "candidate_count",
      "search_stats", "checked_count", "current_page_item_count", "page_index", "page_size", "revision", "deferred", "reason_code",
    ]) ||
    !exactKeys(query, ["provider", "dataset", "start", "end", "target_count"])
  ) return null;
  const queryProvider = optionalString(query.provider);
  const queryDataset = optionalString(query.dataset);
  const queryStart = optionalString(query.start);
  const queryEnd = optionalString(query.end);
  const queryTargetCount = query.target_count === null ? null : safeInteger(query.target_count);
  const candidateCount = safeInteger(source.candidate_count);
  const checkedCount = safeInteger(source.checked_count);
  const currentPageItemCount = safeInteger(source.current_page_item_count);
  const pageIndex = safeInteger(source.page_index);
  const pageSize = safeInteger(source.page_size);
  const revision = safeInteger(source.revision);
  const deferred = boolean(source.deferred);
  const reasonCode = source.reason_code === null ? null : source.reason_code === "candidate_payload_deferred"
    ? "candidate_payload_deferred"
    : undefined;
  if (
    !string(source.batch_id) || !string(source.provider_id) || !string(source.created_at) || !string(source.aoi_name) ||
    queryProvider === undefined || queryDataset === undefined || queryStart === undefined || queryEnd === undefined ||
    queryTargetCount === null && query.target_count !== null ||
    queryTargetCount !== null && (queryTargetCount < 1 || queryTargetCount > 10_000) ||
    candidateCount === null || checkedCount === null || currentPageItemCount === null || pageIndex === null || pageSize === null || pageSize < 1 || revision === null ||
    deferred === null || reasonCode === undefined
  ) return null;
  const pageOffset = pageIndex * pageSize;
  const expectedCurrentPageCount = Number.isSafeInteger(pageOffset)
    ? pageOffset >= candidateCount ? 0 : Math.min(pageSize, candidateCount - pageOffset)
    : null;
  const statsValue = source.search_stats === null ? null : record(source.search_stats);
  const sourceLabel = statsValue?.source;
  const sourceAllowed = sourceLabel === "ASF" || sourceLabel === "ASF_BATCH" || sourceLabel === "ASF_CMR" || sourceLabel === "TYPED_CATALOG";
  const matchedCount = statsValue?.matched_count === null ? null : safeInteger(statsValue?.matched_count);
  const providerReturnedCount = safeInteger(statsValue?.provider_returned_count);
  const frozenCandidateCount = safeInteger(statsValue?.frozen_candidate_count);
  const truncated = boolean(statsValue?.truncated);
  const statsHasSourceAttempts = statsValue
    ? Object.prototype.hasOwnProperty.call(statsValue, "source_attempts")
    : false;
  const sourceAttempts = statsValue
    ? statsHasSourceAttempts
      ? Array.isArray(statsValue.source_attempts) && statsValue.source_attempts.length <= 8
        ? statsValue.source_attempts.map(parseWorkbenchSourceAttempt)
        : null
      : []
    : [];
  const finalAttempt = sourceAttempts?.at(-1) ?? null;
  const sourceAttemptsValid = sourceAttempts !== null
    && sourceAttempts.every((attempt) => attempt !== null)
    && (sourceAttempts.length === 0
      || sourceAttempts.slice(0, -1).every((attempt) => attempt?.outcome === "fallback")
        && finalAttempt?.outcome === "succeeded"
        && finalAttempt.provider_id === source.provider_id);
  if (
    source.search_stats !== null && (
      !statsValue || !exactKeys(statsValue, statsHasSourceAttempts
        ? ["matched_count", "provider_returned_count", "frozen_candidate_count", "truncated", "source_attempts", "source"]
        : ["matched_count", "provider_returned_count", "frozen_candidate_count", "truncated", "source"])
      || !sourceAllowed || matchedCount === null && statsValue.matched_count !== null
      || providerReturnedCount === null || frozenCandidateCount === null || truncated === null
      || frozenCandidateCount !== candidateCount || !sourceAttemptsValid
    )
  ) return null;
  if (
    checkedCount > candidateCount
    || expectedCurrentPageCount === null
    || currentPageItemCount !== expectedCurrentPageCount
  ) return null;
  return {
    batch_id: source.batch_id as string,
    provider_id: source.provider_id as string,
    created_at: source.created_at as string,
    aoi_name: source.aoi_name as string,
    query: {
      provider: queryProvider,
      dataset: queryDataset,
      start: queryStart,
      end: queryEnd,
      target_count: queryTargetCount,
    },
    search_stats: statsValue ? {
      matched_count: matchedCount,
      provider_returned_count: providerReturnedCount!,
      frozen_candidate_count: frozenCandidateCount!,
      truncated: truncated!,
      source_attempts: sourceAttempts as CoreWorkbenchSourceAttemptProjection[],
      source: sourceLabel as "ASF" | "ASF_BATCH" | "ASF_CMR" | "TYPED_CATALOG",
    } : null,
    candidate_count: candidateCount,
    checked_count: checkedCount,
    current_page_item_count: currentPageItemCount,
    page_index: pageIndex,
    page_size: pageSize,
    revision,
    deferred,
    reason_code: reasonCode,
  };
}

export function parseCoreRestore(value: unknown): CoreRestoreProjection | null {
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, [
      "readiness", "workspace_context", "region_aoi", "region_aois", "tasks",
      "dem_source_identity", "legacy_task_warnings", "workbench",
    ]) ||
    Array.isArray(source.candidates)
  ) return null;
  const readiness = record(source.readiness);
  const workspaceContext = nullableRecord(source.workspace_context);
  const regionAoi = nullableRecord(source.region_aoi);
  const regionAois = Array.isArray(source.region_aois)
    ? source.region_aois.map(record)
    : null;
  const demSourceIdentity = parseDemSourceIdentity(source.dem_source_identity);
  const tasks = Array.isArray(source.tasks) ? source.tasks.map(parseCoreTaskSummary) : null;
  const warnings = Array.isArray(source.legacy_task_warnings) ? source.legacy_task_warnings.map(parseWarning) : null;
  const workbench = parseWorkbenchSummary(source.workbench);
  if (
    !readiness ||
    !exactKeys(readiness, ["schema_version", "ready"]) ||
    safeInteger(readiness.schema_version) === null ||
    boolean(readiness.ready) === null ||
    workspaceContext === undefined ||
    regionAoi === undefined ||
    !regionAois ||
    regionAois.some((item) => item === null) ||
    demSourceIdentity === undefined ||
    !tasks ||
    tasks.some((item) => item === null) ||
    !warnings ||
    warnings.some((item) => item === null) ||
    source.workbench !== null && workbench === null
  ) return null;
  return {
    readiness: {
      schema_version: safeInteger(readiness.schema_version)!,
      ready: boolean(readiness.ready)!,
    },
    workspace_context: workspaceContext,
    region_aoi: regionAoi,
    region_aois: regionAois as JsonRecord[],
    dem_source_identity: demSourceIdentity,
    tasks: tasks as CoreTaskSummaryProjection[],
    legacy_task_warnings: warnings as CoreLegacyTaskWarningProjection[],
    workbench,
  };
}

function parsePageItem(value: unknown): WorkbenchPageItemProjection | null {
  const source = record(value);
  const item = source && record(source.item);
  if (
    !source ||
    !item ||
    !exactKeys(source, ["ordinal", "item", "checked"]) ||
    !exactKeys(item, [
      "item_id", "provider_id", "display_name", "acquisition_datetime", "footprint_geojson",
      "metadata", "payload", "asset_access", "execution_policy",
    ])
  ) return null;
  const ordinal = safeInteger(source.ordinal);
  const itemId = string(item.item_id);
  const providerId = string(item.provider_id);
  const displayName = string(item.display_name);
  const acquisitionDatetime = optionalString(item.acquisition_datetime);
  const footprint = item.footprint_geojson === null
    ? null
    : parseCanonicalGeometry(item.footprint_geojson, MAX_CANONICAL_GEOMETRY_BYTES) ?? undefined;
  const metadata = record(item.metadata);
  const payload = record(item.payload);
  const assetAccess = nullableRecord(item.asset_access);
  const executionPolicy = nullableRecord(item.execution_policy);
  const checked = boolean(source.checked);
  if (
    ordinal === null || !itemId || !providerId || !displayName || acquisitionDatetime === undefined ||
    footprint === undefined || !metadata || !payload || assetAccess === undefined || executionPolicy === undefined ||
    checked === null
  ) return null;
  return {
    ordinal,
    item: {
      item_id: itemId,
      provider_id: providerId,
      display_name: displayName,
      acquisition_datetime: acquisitionDatetime,
      footprint_geojson: footprint,
      metadata,
      payload,
      asset_access: assetAccess,
      execution_policy: executionPolicy,
    },
    checked,
  };
}

function nullableNonEmptyString(value: unknown): string | null | undefined {
  if (value === undefined || value === null) return null;
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function parseTaskItemLocalReuse(value: unknown): TaskItemLocalReuseProjection | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["disposition", "grade", "conflict_reason", "conflict_file_name"])) return undefined;
  const disposition = enumValue(source.disposition, LOCAL_REUSE_DISPOSITIONS);
  const grade = source.grade == null ? null : enumValue(source.grade, LOCAL_REUSE_GRADES);
  const conflictReason = nullableNonEmptyString(source.conflict_reason);
  const conflictFileName = nullableNonEmptyString(source.conflict_file_name);
  if (disposition === null || grade === null && source.grade != null || conflictReason === undefined || conflictFileName === undefined) {
    return undefined;
  }
  return { disposition, grade, conflict_reason: conflictReason, conflict_file_name: conflictFileName };
}

function parseOrbitRecheck(value: unknown): OrbitRecheckInfo | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["orbit_type", "state", "attempt", "next_check_at", "deadline_at", "expected_file_name"])) return undefined;
  const state = enumValue(source.state, ["waiting", "exhausted", "cancelled"] as const);
  const attempt = safeInteger(source.attempt);
  const nextCheckAt = string(source.next_check_at);
  const deadlineAt = string(source.deadline_at);
  const expectedFileName = optionalString(source.expected_file_name);
  if (source.orbit_type !== "POEORB" || state === null || attempt === null || !nextCheckAt || !deadlineAt || expectedFileName === undefined) return undefined;
  return {
    orbit_type: "POEORB",
    state,
    attempt,
    next_check_at: nextCheckAt,
    deadline_at: deadlineAt,
    expected_file_name: expectedFileName,
  };
}

function parseTaskPageItem(value: unknown): TaskPageItemProjection | null {
  const source = record(value);
  const pageItem = source
    ? parsePageItem({ ordinal: source.ordinal, item: source.item, checked: source.checked })
    : null;
  const status = enumValue(source?.status, TASK_ITEM_STATUSES);
  const bytesCompleted = safeInteger(source?.bytes_completed);
  const totalBytes = source?.total_bytes === null ? null : safeInteger(source?.total_bytes);
  const sourceTotalBytes = source?.source_total_bytes == null ? null : safeInteger(source.source_total_bytes);
  const bytesPerSecond = typeof source?.bytes_per_second === "number" && Number.isFinite(source.bytes_per_second)
    && source.bytes_per_second >= 0
    ? source.bytes_per_second
    : null;
  const attempt = safeInteger(source?.attempt);
  const destination = optionalString(source?.destination);
  const failureValue = source?.failure === null ? null : record(source?.failure);
  const failure = failureValue === null
    ? null
    : failureValue && string(failureValue.code) && string(failureValue.reason_zh) && boolean(failureValue.retryable) !== null
      ? {
          code: failureValue.code as string,
          reason_zh: failureValue.reason_zh as string,
          retryable: failureValue.retryable as boolean,
        }
      : undefined;
  const finishedAt = optionalString(source?.finished_at);
  const retryable = boolean(source?.retryable);
  const orbitRecheck = parseOrbitRecheck(source?.orbit_recheck);
  const localReuse = parseTaskItemLocalReuse(source?.local_reuse);
  if (
    !source ||
    !exactKeys(source, [
      "ordinal", "item", "checked", "status", "bytes_completed", "total_bytes", "bytes_per_second",
      "attempt", "destination", "failure", "finished_at", "retryable", "source_total_bytes", "local_reuse", "orbit_recheck",
    ]) ||
    !pageItem || status === null || bytesCompleted === null || totalBytes === undefined || bytesPerSecond === null ||
    attempt === null || destination === undefined || failure === undefined || finishedAt === undefined || retryable === null ||
    source.source_total_bytes != null && sourceTotalBytes === null ||
    localReuse === undefined || orbitRecheck === undefined
  ) return null;
  return {
    ...pageItem,
    status,
    bytes_completed: bytesCompleted,
    total_bytes: totalBytes,
    source_total_bytes: sourceTotalBytes,
    bytes_per_second: bytesPerSecond,
    attempt,
    destination,
    failure,
    finished_at: finishedAt,
    retryable,
    orbit_recheck: orbitRecheck,
    local_reuse: localReuse,
  };
}

function parseWorkbenchPageRender(
  value: unknown,
  pageItems: WorkbenchPageItemProjection[],
  aoiEnvelope?: WrappedAoiEnvelope,
): WorkbenchPageRenderAuthorityProjection | null | undefined {
  if (value === undefined || value === null) return null;
  const source = record(value);
  const serializedBytes = serializedJsonBytes(value);
  if (serializedBytes === null || serializedBytes > MAX_DISPLAY_RENDER_PAYLOAD_BYTES) return undefined;
  if (!source || !exactKeys(source, ["region_id", "revision", "items"])) return undefined;
  const regionId = string(source.region_id);
  const revision = safeInteger(source.revision);
  const items = Array.isArray(source.items) ? source.items.map((item) => {
    const row = record(item);
    if (!row || !exactKeys(row, ["item_id", "render_geometry", "render_bbox"])) return null;
    const itemId = string(row.item_id);
    const geometry = row.render_geometry === null ? null : parseDisplayGeometry(row.render_geometry);
    const bbox = row.render_bbox === null ? null : parseDisplayBbox(row.render_bbox);
    const geometryWithinBbox = geometry && bbox ? displayGeometryWithinBbox(geometry, bbox) : true;
    const paired = row.render_geometry === null && row.render_bbox === null || geometry !== null && bbox !== null;
    const pageItem = pageItems.find((candidate) => candidate.item.item_id === itemId);
    const canonical = pageItem?.item.footprint_geojson ?? null;
    const canonicalMatches = geometry === null || canonical !== null && geometryWorldCopyOfCanonical(geometry, canonical, false);
    const relevant = geometry === null || !aoiEnvelope || displayGeometryCopiesRelevant(geometry, aoiEnvelope, false);
    return itemId && pageItem && paired && (row.render_geometry === null || geometry) && (row.render_bbox === null || bbox) &&
      geometryWithinBbox && canonicalMatches && relevant
      ? { item_id: itemId, render_geometry: geometry, render_bbox: bbox }
      : null;
  }) : null;
  if (
    !regionId || revision === null || !items || items.length !== pageItems.length ||
    items.some((item) => item === null) ||
    new Set((items as WorkbenchPageRenderItemProjection[]).map((item) => item.item_id)).size !== items.length ||
    (items as WorkbenchPageRenderItemProjection[]).some((item) => !pageItems.some((page) => page.item.item_id === item.item_id))
  ) return undefined;
  return { region_id: regionId, revision, items: items as WorkbenchPageRenderItemProjection[] };
}

export function parseWorkbenchPage(value: unknown, aoiEnvelope?: WrappedAoiEnvelope): WorkbenchPageProjection | null {
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, ["batch_id", "revision", "page_index", "page_size", "total_count", "checked_count", "items", "render"])
  ) return null;
  const batchId = string(source.batch_id);
  const revision = safeInteger(source.revision);
  const pageIndex = safeInteger(source.page_index);
  const pageSize = safeInteger(source.page_size);
  const totalCount = safeInteger(source.total_count);
  const checkedCount = safeInteger(source.checked_count);
  const items = Array.isArray(source.items) ? source.items.map(parsePageItem) : null;
  if (
    !batchId || revision === null || pageIndex === null || pageSize === null || pageSize < 1 ||
    pageSize > MAX_UI_WORKBENCH_PAGE_SIZE || totalCount === null || checkedCount === null ||
    !items || items.length > MAX_UI_WORKBENCH_PAGE_SIZE || items.some((item) => item === null) || items.length > pageSize
  ) return null;
  const render = parseWorkbenchPageRender(source.render, items as WorkbenchPageItemProjection[], aoiEnvelope);
  if (render === undefined) return null;
  return {
    batch_id: batchId,
    revision,
    page_index: pageIndex,
    page_size: pageSize,
    total_count: totalCount,
    checked_count: checkedCount,
    items: items as WorkbenchPageItemProjection[],
    render,
  };
}

export function parseTaskPage(value: unknown): TaskPageProjection | null {
  const source = record(value);
  if (
    !source ||
    !exactKeys(source, ["task_id", "revision", "page_index", "page_size", "total_count", "items", "source_total_bytes", "network_bytes_received"])
  ) return null;
  const taskId = string(source.task_id);
  const revision = safeInteger(source.revision);
  const pageIndex = safeInteger(source.page_index);
  const pageSize = safeInteger(source.page_size);
  const totalCount = safeInteger(source.total_count);
  const sourceTotalBytes = source.source_total_bytes == null ? null : safeInteger(source.source_total_bytes);
  const networkBytesReceived = source.network_bytes_received == null ? null : safeInteger(source.network_bytes_received);
  const items = Array.isArray(source.items) ? source.items.map(parseTaskPageItem) : null;
  if (
    !taskId || revision === null || pageIndex === null || pageSize === null || pageSize < 1 ||
    pageSize > MAX_UI_TASK_PAGE_SIZE || totalCount === null ||
    source.source_total_bytes != null && sourceTotalBytes === null ||
    source.network_bytes_received != null && networkBytesReceived === null ||
    !items || items.length > MAX_UI_TASK_PAGE_SIZE || items.some((item) => item === null) || items.length > pageSize
  ) return null;
  return {
    task_id: taskId,
    revision,
    page_index: pageIndex,
    page_size: pageSize,
    total_count: totalCount,
    source_total_bytes: sourceTotalBytes,
    network_bytes_received: networkBytesReceived,
    items: items as TaskPageItemProjection[],
  };
}

function invalidSnapshot(message: string): { ok: false; error: string; code: "V3_APPLICATION_INVALID_SNAPSHOT" } {
  return { ok: false, error: message, code: "V3_APPLICATION_INVALID_SNAPSHOT" };
}

export function projectCoreRestore(snapshot: DesktopCoreRestoreSnapshot): CoreRestoreProjectionResult {
  if (!snapshot.ok) return snapshot;
  const core = parseCoreRestore(snapshot.core);
  const region = core?.region_aoi;
  const regionGeometry = region === null ? undefined : region?.geometry;
  const aoiDisplay = parseCoreAoiDisplay(snapshot.aoi_display, regionGeometry);
  const regionId = region === null ? null : string(region?.region_id);
  const regionRevision = region === null ? null : safeInteger(region?.revision);
  const authorityMatchesRegion = aoiDisplay === null || aoiDisplay === undefined
    || (regionId !== null && regionRevision !== null && regionGeometry !== undefined
      && aoiDisplay.region_id === regionId && aoiDisplay.revision === regionRevision);
  return core && aoiDisplay !== undefined && authorityMatchesRegion
    ? { ok: true, core, aoi_display: aoiDisplay }
    : invalidSnapshot("CoreRestore payload is invalid or exceeds the bounded contract");
}

export function projectWorkbenchPage(
  snapshot: DesktopWorkbenchPageSnapshot,
  aoiEnvelope?: WrappedAoiEnvelope,
): WorkbenchPageProjectionResult {
  if (!snapshot.ok) return snapshot;
  const parsedPage = parseWorkbenchPage(snapshot.page, aoiEnvelope);
  if (!parsedPage) return invalidSnapshot("Workbench page payload is invalid or exceeds the bounded contract");
  const render = snapshot.render === undefined
    ? parsedPage.render
    : parseWorkbenchPageRender(snapshot.render, parsedPage.items, aoiEnvelope);
  if (render === undefined) return invalidSnapshot("Workbench page render payload is invalid or exceeds the bounded contract");
  const page = { ...parsedPage, render };
  return page
    ? { ok: true, page, render }
    : invalidSnapshot("Workbench page payload is invalid or exceeds the bounded contract");
}

export function coreRestoreObject(core: CoreRestoreProjection): DesktopJsonObject {
  return core as unknown as DesktopJsonObject;
}
