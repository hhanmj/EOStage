import type { Bbox, SceneRow } from "@/lib/bridge";
import type { AgentStreamEvent } from "@/lib/generated/v3-contracts";
import type { RowTaskState } from "@/shell/api/types";
import { canonicalSourceDisplayName } from "@/lib/dataSourceCatalogLogic";

const LEGACY_OR_ALIAS_RESOURCE_IDS: Readonly<Record<string, string>> = {
  "sar.sentinel1.slc.v1": "science.sentinel-1.standard.v1",
  "optical.copernicus.s2.sr.v1": "science.sentinel-2.l2a.v1",
  "optical.sentinel2.l2a.v1": "science.sentinel-2.l2a.v1",
  "optical.sentinel2.l1c.v1": "science.sentinel-2.l1c.v1",
  "optical.landsat.c2-l2.v1": "science.landsat.c2-l2.v1",
  "dem.copernicus.glo30.v1": "dem.cop30.v1",
  "dem.copernicus.glo90.v1": "dem.cop90.v1",
};

const RAW_PROVIDER_ONLY_IDS = new Set([
  "asf.catalog",
  "asf.nasa.earthdata",
  "mpc.stac",
  "planetary-computer",
  "usgs.eros.m2m",
  "isro.nrsc.bhoonidhi",
  "cdse.copernicus",
  "google.earth-engine",
]);

const CONCISE_DATASET_LABELS: Readonly<Record<string, string>> = {
  "science.sentinel-1.standard.v1": "Sentinel-1",
  "science.sentinel-1.slc-burst.v1": "Sentinel-1 Burst",
  "science.sentinel-2.l2a.v1": "Sentinel-2 L2A",
  "science.sentinel-2.l1c.v1": "Sentinel-2 L1C",
  "science.landsat.c2-l2.v1": "Landsat C2 L2",
  "science.hls.l30.v1": "HLS L30",
  "science.hls.l30.v2": "HLS L30",
  "science.hls.s30.v1": "HLS S30",
  "science.hls.s30.v2": "HLS S30",
  "science.nisar.l.rslc.validated.v1": "NISAR RSLC (Validated)",
  "science.nisar.l.rslc.provisional.v1": "NISAR RSLC (Provisional)",
  "science.nisar.s.ssar.rslc.v1": "NISAR S-SAR RSLC",
  "science.alos-2.palsar-2.scansar-l1.1.v1": "ALOS-2 ScanSAR",
  "science.alos.palsar.v1": "ALOS PALSAR",
  "science.alos.palsar.l1.0-1.5.v1": "ALOS PALSAR",
  "science.opera.disp-s1.v1": "OPERA Sentinel-1",
  "science.aria.s1-gunw.v1": "ARIA S1 GUNW",
  "science.seasat.sar.v1": "SEASAT SAR",
  "dem.cop30.v1": "Copernicus DEM 30m",
  "dem.cop90.v1": "Copernicus DEM 90m",
  "dem.nasadem.v1": "NASADEM 30m",
  "dem.aw3d30.v1": "ALOS World 3D 30m",
  "dem.aw3d30-ellipsoidal.v1": "ALOS World 3D 30m",
  "dem.nisar-dem.v1": "NISAR DEM 30m",
  "dem.fabdem.v1": "FABDEM 30m",
  "dem.gem15-dtm.v1": "GEM-15 DTM 15m",
  "dem.gem15-dsm.v1": "GEM-15 DSM 15m",
  "dem.srtm-gl1.v1": "SRTM GL1 30m",
  "dem.srtm-gl1-ellipsoidal.v1": "SRTM GL1 30m",
  "dem.srtm-gl3.v1": "SRTM GL3 90m",
  "dem.aster-gdem.v3": "ASTER GDEM 30m",
};

const ASF_NISAR_VALIDATED_COLLECTIONS = new Set([
  "C3622236985-ASF",
  "C3622244601-ASF",
  "C3622241360-ASF",
  "C3622247503-ASF",
  "C3622250865-ASF",
  "C3622229381-ASF",
  "C3622237369-ASF",
  "C3622241997-ASF",
]);

const ASF_NISAR_PROVISIONAL_COLLECTIONS = new Set([
  "C2853145197-ASF",
  "C2854332392-ASF",
  "C2854338529-ASF",
  "C2854335566-ASF",
  "C2854341702-ASF",
  "C2853147928-ASF",
  "C2853153429-ASF",
  "C2853156054-ASF",
  "C2854344945-ASF",
]);

function extractSceneHints(s: SceneRow): {
  combinedUpper: string;
  collectionId: string;
  shortName: string;
  tier: "validated" | "provisional" | null;
} {
  const meta = (s.native_metadata || {}) as Record<string, unknown>;
  const props = (
    meta.properties && typeof meta.properties === "object" && !Array.isArray(meta.properties)
      ? meta.properties
      : {}
  ) as Record<string, unknown>;
  const rawScene = s as unknown as Record<string, unknown>;
  const rawProps = (
    rawScene.properties && typeof rawScene.properties === "object" && !Array.isArray(rawScene.properties)
      ? rawScene.properties
      : {}
  ) as Record<string, unknown>;
  const pickStr = (v: unknown) => (typeof v === "string" ? v.trim() : "");
  const collectionId = (
    pickStr(meta.collection_concept_id) ||
    pickStr(meta.collection_id) ||
    pickStr(props.collection_concept_id) ||
    pickStr(props.collection_id) ||
    pickStr(rawProps.collection_concept_id) ||
    pickStr(rawProps.collection_id) ||
    pickStr(rawScene.collection_concept_id)
  ).toUpperCase();
  const shortName = (
    pickStr(meta.collection_short_name) ||
    pickStr(meta.short_name) ||
    pickStr(props.collection_short_name) ||
    pickStr(props.short_name) ||
    pickStr(rawProps.collection_short_name) ||
    pickStr(rawProps.short_name) ||
    pickStr(rawScene.collection_short_name)
  ).toUpperCase();
  const explicitTier = (
    pickStr(meta.release_tier) ||
    pickStr(props.release_tier) ||
    pickStr(rawProps.release_tier) ||
    pickStr(rawScene.release_tier) ||
    pickStr(meta.tier) ||
    pickStr(props.tier) ||
    pickStr(rawProps.tier) ||
    pickStr(meta.quality_status) ||
    pickStr(props.quality_status) ||
    pickStr(rawProps.quality_status)
  ).toLowerCase();
  const combinedUpper = [
    s.scene_id || "",
    s.product_key || "",
    s.file_name || "",
    s.platform || "",
    pickStr(meta.dataset),
    pickStr(props.dataset),
    collectionId,
    shortName,
  ]
    .join(" ")
    .toUpperCase();

  let tier: "validated" | "provisional" | null = null;
  // 1. Authoritative backend release_tier / tier takes precedence (NASA NISAR Validated granules
  //    also contain `_L1_PR_` / `_L2_PR_` where PR = Product, so filename `_PR_` NEVER implies Provisional).
  if (explicitTier === "validated" || explicitTier.includes("validated") || explicitTier.includes("official")) {
    tier = "validated";
  } else if (explicitTier === "provisional" || explicitTier.includes("provisional") || explicitTier.includes("beta")) {
    tier = "provisional";
  } else if (ASF_NISAR_VALIDATED_COLLECTIONS.has(collectionId)) {
    tier = "validated";
  } else if (ASF_NISAR_PROVISIONAL_COLLECTIONS.has(collectionId)) {
    tier = "provisional";
  } else if (shortName.includes("PROVISIONAL") || shortName.includes("BETA")) {
    tier = "provisional";
  } else if (/^NISAR_L[123]_[A-Z0-9_]+_V\d+$/.test(shortName)) {
    tier = "validated";
  }
  return { combinedUpper, collectionId, shortName, tier };
}

export function detectNisarSceneReleaseTier(
  scene: SceneRow
): "validated" | "provisional" | null {
  const hints = extractSceneHints(scene);
  return hints.combinedUpper.includes("NISAR") ? hints.tier : null;
}

function inferNisarLevelFromScene(scene: SceneRow, combinedUpper: string): string {
  const ptype = (scene.product_type || "").trim().toUpperCase();
  if (/^(RSLC|GSLC|GCOV|GUNW|GOFF|RIFG|RUNW|ROFF|SME2)$/.test(ptype)) {
    return ptype;
  }
  const m = /\bNISAR_L[123]_(?:PR_)?(RSLC|GSLC|GCOV|GUNW|GOFF|RIFG|RUNW|ROFF|SME2)\b/.exec(combinedUpper)
    ?? /_(RSLC|GSLC|GCOV|GUNW|GOFF|RIFG|RUNW|ROFF|SME2)_/.exec(combinedUpper);
  return m ? m[1] : "RSLC";
}

function inferCanonicalResourceFromScenes(scenes: readonly SceneRow[]): string | null {
  for (const s of scenes) {
    const { combinedUpper, collectionId, tier } = extractSceneHints(s);
    if (
      combinedUpper.includes("::COLLECTION::LANDSAT-C2-L2::") ||
      combinedUpper.includes("SCIENCE.LANDSAT.C2-L2") ||
      /\bL[CETM]0[45789]_L2/.test(combinedUpper)
    ) {
      return "science.landsat.c2-l2.v1";
    }
    if (combinedUpper.includes("::COLLECTION::SENTINEL-2-L1C::") || /\bS2[ABC]_MSIL1C_/.test(combinedUpper)) {
      return "science.sentinel-2.l1c.v1";
    }
    if (
      combinedUpper.includes("::COLLECTION::SENTINEL-2-L2A::") ||
      combinedUpper.includes("OPTICAL.COPERNICUS.S2") ||
      /\bS2[ABC]_MSIL2A_/.test(combinedUpper)
    ) {
      return "science.sentinel-2.l2a.v1";
    }
    if (combinedUpper.includes("NISAR_S_") || (combinedUpper.includes("NISAR") && combinedUpper.includes("SSAR"))) {
      const ptype = (s.product_type || "").trim().toUpperCase();
      return ptype ? `science.nisar.s.${ptype.toLowerCase()}.v1` : "science.nisar.s.ssar.rslc.v1";
    }
    if (
      combinedUpper.includes("NISAR_") ||
      combinedUpper.includes("NISAR") ||
      ASF_NISAR_VALIDATED_COLLECTIONS.has(collectionId) ||
      ASF_NISAR_PROVISIONAL_COLLECTIONS.has(collectionId)
    ) {
      const level = inferNisarLevelFromScene(s, combinedUpper).toLowerCase();
      if (tier === "provisional") {
        return `science.nisar.l.${level}.provisional.v1`;
      }
      if (tier === "validated") {
        return `science.nisar.l.${level}.validated.v1`;
      }
      return `science.nisar.l.${level}.v1`;
    }
    if (
      collectionId === "C3315903479-ASF" ||
      combinedUpper.includes("ALOS2") ||
      combinedUpper.includes("PALSAR-2")
    ) {
      return "science.alos-2.palsar-2.scansar-l1.1.v1";
    }
    if (combinedUpper.includes("ALPSRP") || combinedUpper.includes("ALOS PALSAR")) {
      return "science.alos.palsar.v1";
    }
    if (collectionId === "C2834279090-ASF" || combinedUpper.includes("OPERA_L")) {
      return "science.opera.disp-s1.v1";
    }
    if (collectionId === "C1595771383-ASF" || combinedUpper.includes("S1-GUNW")) {
      return "science.aria.s1-gunw.v1";
    }
    if (/\bS1_.*_BURST\b/.test(combinedUpper) || combinedUpper.includes("SLC-BURST")) {
      return "science.sentinel-1.slc-burst.v1";
    }
    if (
      combinedUpper.includes("::COLLECTION::SENTINEL-1::") ||
      /\bS1[ABCD]_(IW|EW|SM)_/.test(combinedUpper) ||
      combinedUpper.includes("SENTINEL-1")
    ) {
      return "science.sentinel-1.standard.v1";
    }
    if (combinedUpper.includes("::COLLECTION::COP-DEM-GLO-30::")) return "dem.cop30.v1";
    if (combinedUpper.includes("::COLLECTION::COP-DEM-GLO-90::")) return "dem.cop90.v1";
    if (combinedUpper.includes("::COLLECTION::HLS-L30::") || /\bHLS\.L30\./.test(combinedUpper)) {
      return "science.hls.l30.v2";
    }
    if (combinedUpper.includes("::COLLECTION::HLS-S30::") || /\bHLS\.S30\./.test(combinedUpper)) {
      return "science.hls.s30.v2";
    }
    if (combinedUpper.includes("::COLLECTION::NASADEM::") || combinedUpper.includes("NASADEM")) {
      return "dem.nasadem.v1";
    }
    if (combinedUpper.includes("::COLLECTION::FABDEM::") || combinedUpper.includes("FABDEM")) {
      return "dem.fabdem.v1";
    }
    if (combinedUpper.includes("::COLLECTION::ASTER-GDEM::") || combinedUpper.includes("ASTGTM")) {
      return "dem.aster-gdem.v3";
    }
    if (combinedUpper.includes("::COLLECTION::AW3D30::") || combinedUpper.includes("ALPSMLC30")) {
      return "dem.aw3d30.v1";
    }
    if (combinedUpper.includes("::COLLECTION::SRTM-GL1::") || combinedUpper.includes("SRTMGL1")) {
      return "dem.srtm-gl1.v1";
    }
  }
  return null;
}

export function canonicalAgentResourceKey(
  resourceId: string | null | undefined,
  scenes: readonly SceneRow[] = []
): string | null {
  const trimmed = typeof resourceId === "string" ? resourceId.trim() : "";
  const lower = trimmed.toLowerCase();
  if (trimmed && !RAW_PROVIDER_ONLY_IDS.has(lower)) {
    const mapped = LEGACY_OR_ALIAS_RESOURCE_IDS[lower] ?? trimmed;
    if (mapped.startsWith("science.nisar.") && scenes.length > 0) {
      const ptype = (scenes[0]?.product_type || "").trim().toUpperCase();
      const hints = extractSceneHints(scenes[0]);
      const level = /^(RSLC|GSLC|GCOV|GUNW|GOFF|RIFG|RUNW|ROFF|SME2)$/.test(ptype)
        ? ptype.toLowerCase()
        : inferNisarLevelFromScene(scenes[0], hints.combinedUpper).toLowerCase();
      const band = mapped.includes(".s.") ? "s" : "l";
      if (band === "s") {
        return `science.nisar.s.ssar.${level}.v1`;
      }
      const tiers = new Set(scenes.map((sc) => detectNisarSceneReleaseTier(sc)).filter(Boolean));
      if (tiers.size === 1) {
        const onlyTier = Array.from(tiers)[0];
        return `science.nisar.l.${level}.${onlyTier}.v1`;
      }
      if (tiers.size > 1) {
        return `science.nisar.l.${level}.v1`;
      }
      if (mapped.includes(".provisional.")) {
        return `science.nisar.l.${level}.provisional.v1`;
      }
      if (mapped.includes(".validated.")) {
        return `science.nisar.l.${level}.validated.v1`;
      }
      return `science.nisar.l.${level}.v1`;
    }
    return mapped;
  }
  return inferCanonicalResourceFromScenes(scenes);
}

function isOpaqueIdentifierText(value: string): boolean {
  const s = value.trim();
  if (!s) return true;
  if (/^[a-z0-9_-]+(\.[a-z0-9_-]+){1,}$/i.test(s)) return true;
  if (s.startsWith("PROVIDER::")) return true;
  return RAW_PROVIDER_ONLY_IDS.has(s.toLowerCase());
}

export function canonicalAgentDatasetShortName(
  resourceId: string | null | undefined,
  scenes: readonly SceneRow[] = [],
  sourceNames?: Readonly<Record<string, string>>,
  fallbackLabel?: string | null
): string {
  const canonicalKey = canonicalAgentResourceKey(resourceId, scenes);
  const firstScene = scenes[0];
  const ptype = (firstScene?.product_type || "").trim().toUpperCase();

  if (canonicalKey) {
    if (canonicalKey === "science.sentinel-1.standard.v1") {
      if (ptype === "SLC" || /_SLC_/.test((firstScene?.scene_id || firstScene?.file_name || "").toUpperCase())) {
        return "Sentinel-1 SLC";
      }
      if (ptype === "GRD" || /_GRD/.test((firstScene?.scene_id || firstScene?.file_name || "").toUpperCase())) {
        return "Sentinel-1 GRD";
      }
      return "Sentinel-1";
    }
    const nisarMatch = canonicalKey.match(
      /^science\.nisar\.([ls])\.(?:ssar\.)?(rslc|gslc|gcov|gunw|goff|rifg|runw|roff|sme2)\b(?:\.(validated|provisional))?/i
    );
    if (nisarMatch) {
      const band = nisarMatch[1].toUpperCase();
      const level = /^(RSLC|GSLC|GCOV|GUNW|GOFF|RIFG|RUNW|ROFF|SME2)$/.test(ptype)
        ? ptype
        : nisarMatch[2].toUpperCase();
      if (band === "S") {
        return `NISAR S-SAR ${level}`;
      }
      const keyTier = nisarMatch[3]?.toLowerCase() ?? null;
      const tiers = new Set(scenes.map((sc) => detectNisarSceneReleaseTier(sc)).filter(Boolean));
      const effectiveTier =
        tiers.size === 1
          ? Array.from(tiers)[0]
          : tiers.size === 0
            ? keyTier
            : null;
      if (effectiveTier === "validated") {
        return `NISAR ${level} (Validated)`;
      }
      if (effectiveTier === "provisional") {
        return `NISAR ${level} (Provisional)`;
      }
      return `NISAR L-band ${level}`;
    }
    if (CONCISE_DATASET_LABELS[canonicalKey]) {
      return CONCISE_DATASET_LABELS[canonicalKey];
    }
    const fromSourceMap = sourceNames?.[canonicalKey] || (resourceId ? sourceNames?.[resourceId] : undefined);
    if (fromSourceMap && !isOpaqueIdentifierText(fromSourceMap) && !fromSourceMap.startsWith("batch-")) {
      return fromSourceMap;
    }
    const display = canonicalSourceDisplayName(canonicalKey);
    if (display && !isOpaqueIdentifierText(display)) {
      return display;
    }
  }

  if (fallbackLabel) {
    const clean = fallbackLabel.split(" · ")[0]?.trim() || "";
    if (clean && !isOpaqueIdentifierText(clean) && !clean.startsWith("batch-")) {
      return clean;
    }
  }

  if (firstScene?.platform?.trim()) {
    const plat = firstScene.platform.trim();
    if (!isOpaqueIdentifierText(plat)) {
      return ptype && !plat.toUpperCase().includes(ptype) ? `${plat} ${ptype}` : plat;
    }
  }

  return "检索批次";
}

function extractBboxFromGeoJson(geo: unknown): Bbox | null {
  if (!geo || typeof geo !== "object") return null;
  let minLon = Infinity;
  let minLat = Infinity;
  let maxLon = -Infinity;
  let maxLat = -Infinity;
  const walk = (node: unknown) => {
    if (!Array.isArray(node)) return;
    if (
      node.length >= 2 &&
      typeof node[0] === "number" &&
      typeof node[1] === "number" &&
      Number.isFinite(node[0]) &&
      Number.isFinite(node[1])
    ) {
      if (node[0] < minLon) minLon = node[0];
      if (node[0] > maxLon) maxLon = node[0];
      if (node[1] < minLat) minLat = node[1];
      if (node[1] > maxLat) maxLat = node[1];
      return;
    }
    for (const child of node) walk(child);
  };
  const g = geo as Record<string, unknown>;
  if (g.geometry) return extractBboxFromGeoJson(g.geometry);
  if (Array.isArray(g.features) && g.features.length > 0) {
    for (const f of g.features) {
      const b = extractBboxFromGeoJson(f);
      if (b) {
        minLon = Math.min(minLon, b.west);
        maxLon = Math.max(maxLon, b.east);
        minLat = Math.min(minLat, b.south);
        maxLat = Math.max(maxLat, b.north);
      }
    }
  } else if (g.coordinates) {
    walk(g.coordinates);
  }
  if (
    Number.isFinite(minLon) &&
    Number.isFinite(maxLon) &&
    Number.isFinite(minLat) &&
    Number.isFinite(maxLat)
  ) {
    return { west: minLon, south: minLat, east: maxLon, north: maxLat, crs: "EPSG:4326" };
  }
  return null;
}

export function sceneIntersectsAoiBbox(
  scene: SceneRow,
  aoi: Bbox | null | undefined
): boolean {
  if (
    !aoi ||
    ![aoi.west, aoi.south, aoi.east, aoi.north].every(Number.isFinite) ||
    aoi.west >= aoi.east ||
    aoi.south >= aoi.north
  ) {
    return true;
  }
  const sb =
    scene.footprint_bbox &&
    [
      scene.footprint_bbox.west,
      scene.footprint_bbox.south,
      scene.footprint_bbox.east,
      scene.footprint_bbox.north,
    ].every(Number.isFinite)
      ? scene.footprint_bbox
      : extractBboxFromGeoJson(scene.footprint_geojson);
  if (!sb) return true;
  return !(
    sb.east < aoi.west ||
    sb.west > aoi.east ||
    sb.north < aoi.south ||
    sb.south > aoi.north
  );
}

export function filterBatchCollectionByAoi(
  collection: ProjectedAgentBatchCollection,
  aoi: Bbox | null | undefined
): ProjectedAgentBatchCollection {
  if (
    !aoi ||
    ![aoi.west, aoi.south, aoi.east, aoi.north].every(Number.isFinite) ||
    aoi.west >= aoi.east ||
    aoi.south >= aoi.north
  ) {
    return collection;
  }
  const filteredBatches: AgentSessionBatchEntry[] = [];
  for (const b of collection.batches) {
    const keptScenes = b.scenes.filter((s) => sceneIntersectsAoiBbox(s, aoi));
    const removedCount = b.scenes.length - keptScenes.length;
    if (b.scenes.length > 0 && keptScenes.length === 0) {
      continue;
    }
    const nextTotal =
      removedCount > 0 ? Math.max(keptScenes.length, b.totalCount - removedCount) : b.totalCount;
    filteredBatches.push({
      ...b,
      scenes: keptScenes,
      totalCount: nextTotal,
    });
  }
  return projectAgentSessionBatchCollection(filteredBatches, collection.activeFilter);
}

export interface AgentSessionBatchEntry {
  batchId: string;
  resourceId: string | null;
  revision: number;
  scenes: SceneRow[];
  rowStates: Map<string, RowTaskState>;
  totalCount: number;
  updatedAtMs: number;
}

/**
 * Per-session workspace snapshot persisted in Tauri backend (or in-memory Map fallback
 * during Node/fixture/browser runs without Tauri IPC).
 *
 * Implements strict Requirement #2 + Multi-Dataset Batch Coexistence:
 * - New Agent drafts (`activeSessionId === null`) live-follow Manual Workbench.
 * - Once a session is created or selected (`sessionId !== null`), its AOI, multi-dataset
 *   search batches, and scene results stay isolated from Manual Workbench and other sessions.
 */
export interface AgentSessionWorkspaceState {
  sessionId: string;
  workingAoi: Bbox | null;
  workingGeometry: Record<string, unknown> | null;
  aoiLabel: string;
  batchId: string | null;
  resourceId: string | null;
  scenes?: SceneRow[];
  rowStates?: Map<string, RowTaskState>;
  batchTotalCount?: number;
  selectedSceneId?: string | null;
  batches?: AgentSessionBatchEntry[];
  activeBatchFilter?: "all" | string;
}

export interface AgentSessionWorkspacePatch {
  workingAoi?: Bbox | null;
  workingGeometry?: Record<string, unknown> | null;
  aoiLabel?: string;
  batchId?: string | null;
  resourceId?: string | null;
  scenes?: SceneRow[];
  rowStates?: Map<string, RowTaskState>;
  batchTotalCount?: number;
  selectedSceneId?: string | null;
  batches?: AgentSessionBatchEntry[];
  activeBatchFilter?: "all" | string;
}

const memoryWorkspaceBySession = new Map<string, AgentSessionWorkspaceState>();

function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  return "__TAURI_INTERNALS__" in window || "__TAURI__" in window;
}

async function invokeTauri<T>(command: string, args: Record<string, unknown>): Promise<T | null> {
  if (!isTauriRuntime()) return null;
  try {
    const mod = await import("@tauri-apps/api/core");
    if (typeof mod?.invoke !== "function") return null;
    return (await mod.invoke(command, args)) as T;
  } catch {
    return null;
  }
}

function parseBbox(raw: unknown): Bbox | null {
  if (!raw) return null;
  if (Array.isArray(raw) && raw.length >= 4) {
    const [w, s, e, n] = raw.map(Number);
    if ([w, s, e, n].every(Number.isFinite) && w < e && s < n) {
      return { west: w, south: s, east: e, north: n, crs: "EPSG:4326" };
    }
    return null;
  }
  if (typeof raw === "object") {
    const obj = raw as Record<string, unknown>;
    const w = Number(obj.west);
    const s = Number(obj.south);
    const e = Number(obj.east);
    const n = Number(obj.north);
    if ([w, s, e, n].every(Number.isFinite) && w < e && s < n) {
      return {
        west: w,
        south: s,
        east: e,
        north: n,
        crs: typeof obj.crs === "string" && obj.crs.trim() ? obj.crs : "EPSG:4326",
      };
    }
  }
  return null;
}

function parseGeometry(raw: unknown): Record<string, unknown> | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  return raw as Record<string, unknown>;
}

function normalizeRemoteState(
  sessionId: string,
  raw: unknown,
  cached?: AgentSessionWorkspaceState
): AgentSessionWorkspaceState | null {
  if (!raw || typeof raw !== "object") return cached ?? null;
  const top = raw as Record<string, unknown>;
  const rec =
    top.workspace && typeof top.workspace === "object"
      ? (top.workspace as Record<string, unknown>)
      : top.state && typeof top.state === "object"
        ? (top.state as Record<string, unknown>)
        : top;

  const rawAoiObj =
    rec.working_aoi && typeof rec.working_aoi === "object" && !Array.isArray(rec.working_aoi)
      ? (rec.working_aoi as Record<string, unknown>)
      : null;
  const isExplicitNoneAoi =
    rawAoiObj?.state === "none" || rawAoiObj?.state === "no_aoi" || rawAoiObj?.state === "empty";

  const workingAoi = isExplicitNoneAoi
    ? null
    : (parseBbox(rawAoiObj?.bbox ?? rec.working_aoi ?? rec.workingAoi ?? rec.bbox) ??
        cached?.workingAoi ??
        null);
  const workingGeometry = isExplicitNoneAoi
    ? null
    : (parseGeometry(rawAoiObj?.geometry ?? rec.working_geometry ?? rec.workingGeometry ?? rec.geometry) ??
        cached?.workingGeometry ??
        null);
  const rawLabel = rawAoiObj?.display_name ?? rec.aoi_label ?? rec.aoiLabel ?? rec.display_name;
  const aoiLabel = isExplicitNoneAoi
    ? ""
    : typeof rawLabel === "string"
      ? rawLabel
      : (cached?.aoiLabel ?? "");
  const rawBatch =
    rec.active_batch_id ?? rec.search_batch_id ?? rec.batch_id ?? rec.batchId;
  const batchId =
    typeof rawBatch === "string" && rawBatch.trim().length > 0
      ? rawBatch.trim()
      : rawBatch === null
        ? null
        : (cached?.batchId ?? null);
  const rawResource =
    rec.active_resource_id ?? rec.source_resource_id ?? rec.resource_id ?? rec.resourceId;
  const resourceId =
    typeof rawResource === "string" && rawResource.trim().length > 0
      ? rawResource.trim()
      : rawResource === null
        ? null
        : (cached?.resourceId ?? null);

  return {
    sessionId,
    workingAoi,
    workingGeometry,
    aoiLabel,
    batchId,
    resourceId,
    scenes: cached?.scenes,
    rowStates: cached?.rowStates,
    batchTotalCount: cached?.batchTotalCount,
    selectedSceneId: cached?.selectedSceneId,
    batches: cached?.batches,
    activeBatchFilter: cached?.activeBatchFilter,
  };
}

function buildWorkingAoiRequest(
  aoi: Bbox | null,
  geometry: Record<string, unknown> | null,
  label: string
): Record<string, unknown> | null {
  if (!aoi && !geometry) return null;
  const resolvedGeo =
    geometry && typeof geometry === "object" && !Array.isArray(geometry)
      ? geometry
      : aoi
        ? {
            type: "Polygon",
            coordinates: [
              [
                [aoi.west, aoi.south],
                [aoi.east, aoi.south],
                [aoi.east, aoi.north],
                [aoi.west, aoi.north],
                [aoi.west, aoi.south],
              ],
            ],
          }
        : null;
  if (!resolvedGeo) return null;
  const bboxTuple = aoi ? [aoi.west, aoi.south, aoi.east, aoi.north] : undefined;
  return {
    source_kind: "manual_polygon",
    geometry: resolvedGeo,
    ...(bboxTuple ? { bbox: bboxTuple } : {}),
    crs: "EPSG:4326",
    display_name: label || "工作范围",
  };
}

/** Reads isolated workspace state for an existing Agent session. */
export async function getAgentSessionWorkspaceState(
  sessionId: string
): Promise<AgentSessionWorkspaceState | null> {
  const sid = sessionId.trim();
  if (!sid) return null;
  const cached = memoryWorkspaceBySession.get(sid);
  const remote = await invokeTauri<unknown>("agent_session_workspace_state", {
    sessionId: sid,
  });
  if (remote !== null) {
    const normalized = normalizeRemoteState(sid, remote, cached);
    if (normalized) {
      memoryWorkspaceBySession.set(sid, normalized);
      return normalized;
    }
  }
  return cached ?? null;
}

/**
 * Updates cached & backend-persisted isolated workspace state for `sessionId`.
 * Never mutates Manual Workbench state or other sessions.
 */
export async function setAgentSessionWorkspaceState(
  sessionId: string,
  patch: AgentSessionWorkspacePatch
): Promise<AgentSessionWorkspaceState> {
  const sid = sessionId.trim();
  const prev = memoryWorkspaceBySession.get(sid) ?? {
    sessionId: sid,
    workingAoi: null,
    workingGeometry: null,
    aoiLabel: "",
    batchId: null,
    resourceId: null,
  };
  const next: AgentSessionWorkspaceState = {
    sessionId: sid,
    workingAoi: patch.workingAoi !== undefined ? patch.workingAoi : prev.workingAoi,
    workingGeometry:
      patch.workingGeometry !== undefined ? patch.workingGeometry : prev.workingGeometry,
    aoiLabel: patch.aoiLabel !== undefined ? patch.aoiLabel : prev.aoiLabel,
    batchId: patch.batchId !== undefined ? patch.batchId : prev.batchId,
    resourceId: patch.resourceId !== undefined ? patch.resourceId : prev.resourceId,
    scenes: patch.scenes !== undefined ? patch.scenes : prev.scenes,
    rowStates: patch.rowStates !== undefined ? patch.rowStates : prev.rowStates,
    batchTotalCount:
      patch.batchTotalCount !== undefined ? patch.batchTotalCount : prev.batchTotalCount,
    selectedSceneId:
      patch.selectedSceneId !== undefined ? patch.selectedSceneId : prev.selectedSceneId,
    batches: patch.batches !== undefined ? patch.batches : prev.batches,
    activeBatchFilter:
      patch.activeBatchFilter !== undefined ? patch.activeBatchFilter : prev.activeBatchFilter,
  };
  memoryWorkspaceBySession.set(sid, next);

  if (
    patch.workingAoi !== undefined ||
    patch.workingGeometry !== undefined ||
    patch.aoiLabel !== undefined ||
    patch.batchId !== undefined ||
    patch.resourceId !== undefined
  ) {
    const aoiChanged =
      patch.workingAoi !== undefined ||
      patch.workingGeometry !== undefined ||
      patch.aoiLabel !== undefined;
    const requestPayload = aoiChanged
      ? buildWorkingAoiRequest(next.workingAoi, next.workingGeometry, next.aoiLabel)
      : null;
    const clearAoi = aoiChanged && !next.workingAoi && !next.workingGeometry;
    await invokeTauri("agent_session_set_working_aoi", {
      sessionId: sid,
      request: requestPayload,
      clear: clearAoi ? true : null,
      activeBatchId: next.batchId,
      activeResourceId: next.resourceId,
    });
  }
  return next;
}

/**
 * Accumulates or updates a searched dataset batch inside `sessionId` without wiping
 * previously retrieved datasets in the same session (e.g. Sentinel-1 + Sentinel-2 + NISAR + Landsat).
 */
export async function upsertAgentSessionBatch(
  sessionId: string,
  entry: Omit<AgentSessionBatchEntry, "updatedAtMs"> & {
    updatedAtMs?: number;
    selectThisBatch?: boolean;
  }
): Promise<AgentSessionWorkspaceState> {
  const sid = sessionId.trim();
  const prev = memoryWorkspaceBySession.get(sid);
  const existing = [...(prev?.batches ?? [])];
  const nowMs = entry.updatedAtMs ?? Date.now();
  const resolvedResourceId =
    canonicalAgentResourceKey(entry.resourceId, entry.scenes) ?? entry.resourceId;
  const normalizedEntry: AgentSessionBatchEntry = {
    batchId: entry.batchId,
    resourceId: resolvedResourceId,
    revision: entry.revision,
    scenes: entry.scenes,
    rowStates: entry.rowStates,
    totalCount: Math.max(entry.totalCount, entry.scenes.length),
    updatedAtMs: nowMs,
  };

  const entryCanonicalKey = canonicalAgentResourceKey(
    normalizedEntry.resourceId,
    normalizedEntry.scenes
  );
  const isZeroResult =
    normalizedEntry.scenes.length === 0 && normalizedEntry.totalCount === 0;
  const matchIdx = existing.findIndex((b) => {
    if (b.batchId === normalizedEntry.batchId) return true;
    const bKey = canonicalAgentResourceKey(b.resourceId, b.scenes);
    if (bKey && entryCanonicalKey && bKey === entryCanonicalKey) return true;
    return (
      Boolean(b.resourceId) &&
      Boolean(normalizedEntry.resourceId) &&
      b.resourceId === normalizedEntry.resourceId
    );
  });

  let nextBatches = existing;
  if (matchIdx >= 0) {
    // Do not overwrite an existing non-empty batch for the same resource with an empty 0-result probe
    if (!(isZeroResult && existing[matchIdx].scenes.length > 0)) {
      nextBatches = existing.map((b, idx) => (idx === matchIdx ? normalizedEntry : b));
    }
  } else if (!isZeroResult || existing.length === 0) {
    // Drop any stale 0-scene placeholder when real non-empty batches arrive
    const base = !isZeroResult
      ? existing.filter((b) => b.scenes.length > 0 || b.totalCount > 0)
      : existing;
    nextBatches = [...base, normalizedEntry];
  }

  const nextFilter: "all" | string = entry.selectThisBatch
    ? normalizedEntry.batchId
    : prev?.activeBatchFilter &&
        prev.activeBatchFilter !== "all" &&
        nextBatches.some((b) => b.batchId === prev.activeBatchFilter)
      ? prev.activeBatchFilter
      : "all";

  const projected = projectAgentSessionBatchCollection(nextBatches, nextFilter);
  return setAgentSessionWorkspaceState(sid, {
    batchId: projected.activeBatchId ?? normalizedEntry.batchId,
    resourceId: projected.activeResourceId ?? normalizedEntry.resourceId,
    scenes: projected.scenes,
    rowStates: projected.rowStates,
    batchTotalCount: projected.totalCount,
    batches: nextBatches,
    activeBatchFilter: projected.activeFilter,
  });
}

export function setAgentSessionBatchFilter(
  sessionId: string,
  filter: "all" | string
): AgentSessionWorkspaceState | null {
  const sid = sessionId.trim();
  if (!sid) return null;
  const prev = memoryWorkspaceBySession.get(sid);
  if (!prev || !prev.batches || prev.batches.length === 0) return prev ?? null;
  const projected = projectAgentSessionBatchCollection(prev.batches, filter);
  const next: AgentSessionWorkspaceState = {
    ...prev,
    batchId: projected.activeBatchId ?? prev.batchId,
    resourceId: projected.activeResourceId ?? prev.resourceId,
    scenes: projected.scenes,
    rowStates: projected.rowStates,
    batchTotalCount: projected.totalCount,
    activeBatchFilter: projected.activeFilter,
  };
  memoryWorkspaceBySession.set(sid, next);
  return next;
}

export interface ProjectedAgentBatchCollection {
  batches: AgentSessionBatchEntry[];
  activeFilter: "all" | string;
  activeBatchId: string | null;
  activeResourceId: string | null;
  scenes: SceneRow[];
  rowStates: Map<string, RowTaskState>;
  totalCount: number;
  combinedSceneCount: number;
  combinedTotalCount: number;
}

export function projectAgentSessionBatchCollection(
  rawBatches: readonly AgentSessionBatchEntry[] | undefined,
  requestedFilter?: "all" | string
): ProjectedAgentBatchCollection {
  const list = (rawBatches ?? []).filter((b) => Boolean(b.batchId));
  const nonEmpty = list.filter((b) => b.scenes.length > 0 || b.totalCount > 0);
  const rawEffective = nonEmpty.length > 0 ? nonEmpty : list;
  const dedupedByKey = new Map<string, AgentSessionBatchEntry>();
  for (const b of rawEffective) {
    const resolvedKey = canonicalAgentResourceKey(b.resourceId, b.scenes);
    const shortLabel = canonicalAgentDatasetShortName(b.resourceId, b.scenes);
    const dedupKey =
      resolvedKey ??
      (shortLabel !== "检索批次" ? `label:${shortLabel.toLowerCase()}` : b.batchId);
    const prev = dedupedByKey.get(dedupKey);
    if (!prev || b.updatedAtMs >= prev.updatedAtMs) {
      dedupedByKey.set(dedupKey, {
        ...b,
        resourceId: resolvedKey ?? (isOpaqueIdentifierText(b.resourceId ?? "") ? null : b.resourceId),
      });
    }
  }
  const effectiveBatches = Array.from(dedupedByKey.values());

  const mergedRows = new Map<string, RowTaskState>();
  const combinedScenes: SceneRow[] = [];
  const seenSceneIds = new Set<string>();
  let combinedTotalCount = 0;

  for (const b of effectiveBatches) {
    combinedTotalCount += Math.max(b.totalCount, b.scenes.length);
    for (const [k, v] of b.rowStates.entries()) {
      mergedRows.set(k, v);
    }
    for (const sc of b.scenes) {
      if (!seenSceneIds.has(sc.scene_id)) {
        seenSceneIds.add(sc.scene_id);
        combinedScenes.push(sc);
      }
    }
  }

  if (effectiveBatches.length === 0) {
    return {
      batches: [],
      activeFilter: "all",
      activeBatchId: null,
      activeResourceId: null,
      scenes: [],
      rowStates: mergedRows,
      totalCount: 0,
      combinedSceneCount: 0,
      combinedTotalCount: 0,
    };
  }

  if (effectiveBatches.length === 1) {
    const only = effectiveBatches[0];
    return {
      batches: effectiveBatches,
      activeFilter: requestedFilter === only.batchId ? only.batchId : "all",
      activeBatchId: only.batchId,
      activeResourceId: only.resourceId,
      scenes: only.scenes,
      rowStates: mergedRows,
      totalCount: Math.max(only.totalCount, only.scenes.length),
      combinedSceneCount: combinedScenes.length,
      combinedTotalCount,
    };
  }

  const matchedBatch =
    requestedFilter && requestedFilter !== "all"
      ? effectiveBatches.find((b) => b.batchId === requestedFilter) ?? null
      : null;

  if (matchedBatch) {
    return {
      batches: effectiveBatches,
      activeFilter: matchedBatch.batchId,
      activeBatchId: matchedBatch.batchId,
      activeResourceId: matchedBatch.resourceId,
      scenes: matchedBatch.scenes,
      rowStates: mergedRows,
      totalCount: Math.max(matchedBatch.totalCount, matchedBatch.scenes.length),
      combinedSceneCount: combinedScenes.length,
      combinedTotalCount,
    };
  }

  const latest = effectiveBatches[effectiveBatches.length - 1];
  return {
    batches: effectiveBatches,
    activeFilter: "all",
    activeBatchId: latest.batchId,
    activeResourceId: latest.resourceId,
    scenes: combinedScenes,
    rowStates: mergedRows,
    totalCount: combinedTotalCount,
    combinedSceneCount: combinedScenes.length,
    combinedTotalCount,
  };
}

export function findAgentSceneBatchOwner(
  state: AgentSessionWorkspaceState | null | undefined,
  sceneOrItemId: string
): {
  batchId: string;
  revision: number;
  resourceId: string | null;
  scene: SceneRow;
} | null {
  if (!state || !sceneOrItemId) return null;
  for (const b of state.batches ?? []) {
    const found = b.scenes.find(
      (s) => s.scene_id === sceneOrItemId || s.product_key === sceneOrItemId
    );
    if (found) {
      return {
        batchId: b.batchId,
        revision: b.revision,
        resourceId: b.resourceId,
        scene: found,
      };
    }
  }
  const fallbackScene = state.scenes?.find(
    (s) => s.scene_id === sceneOrItemId || s.product_key === sceneOrItemId
  );
  if (fallbackScene && state.batchId) {
    return {
      batchId: state.batchId,
      revision: 0,
      resourceId: state.resourceId,
      scene: fallbackScene,
    };
  }
  return null;
}

export interface DiscoveredEventBatch {
  batchId: string;
  resourceId: string | null;
  totalCount: number | null;
}

/**
 * Extracts all distinct searched dataset batches recorded in an Agent session's event log
 * so reopening historical multi-dataset turns (e.g. 4 datasets in one prompt) restores all of them.
 */
export function extractSessionBatchesFromEvents(
  events: readonly AgentStreamEvent[]
): DiscoveredEventBatch[] {
  const byBatchId = new Map<string, DiscoveredEventBatch>();
  for (const ev of events) {
    const p = (ev.payload || {}) as Record<string, unknown>;
    if (ev.kind === "workspace_sync" && p.action === "search_updated") {
      const bid = typeof p.batch_id === "string" ? p.batch_id.trim() : "";
      if (!bid) continue;
      const rid = typeof p.resource_id === "string" && p.resource_id.trim() ? p.resource_id.trim() : null;
      const cnt =
        typeof p.total_count === "number"
          ? p.total_count
          : typeof p.loaded_count === "number"
            ? p.loaded_count
            : null;
      const prev = byBatchId.get(bid);
      byBatchId.set(bid, {
        batchId: bid,
        resourceId: rid ?? prev?.resourceId ?? null,
        totalCount: cnt ?? prev?.totalCount ?? null,
      });
      continue;
    }

    if (ev.kind === "tool_round") {
      const bid =
        typeof p.batch_id === "string" && p.batch_id.trim()
          ? p.batch_id.trim()
          : typeof p.search_batch_id === "string" && p.search_batch_id.trim()
            ? p.search_batch_id.trim()
            : "";
      if (bid) {
        const rid = typeof p.resource_id === "string" && p.resource_id.trim() ? p.resource_id.trim() : null;
        const cnt = typeof p.result_count === "number" ? p.result_count : null;
        const prev = byBatchId.get(bid);
        byBatchId.set(bid, {
          batchId: bid,
          resourceId: rid ?? prev?.resourceId ?? null,
          totalCount: cnt ?? prev?.totalCount ?? null,
        });
      }
      continue;
    }

    if (ev.kind === "conclusion" && Array.isArray(p.claims)) {
      for (const claim of p.claims) {
        if (!claim || typeof claim !== "object") continue;
        const refs = (claim as Record<string, unknown>).refs;
        if (!Array.isArray(refs)) continue;
        for (const r of refs) {
          if (!r || typeof r !== "object") continue;
          const refObj = r as Record<string, unknown>;
          if (refObj.kind !== "batch") continue;
          const bid = typeof refObj.id === "string" ? refObj.id.trim() : "";
          if (!bid) continue;
          const cnt = typeof refObj.response_count === "number" ? refObj.response_count : null;
          const prev = byBatchId.get(bid);
          byBatchId.set(bid, {
            batchId: bid,
            resourceId: prev?.resourceId ?? null,
            totalCount: cnt ?? prev?.totalCount ?? null,
          });
        }
      }
    }
  }

  // Keep all batches except 0-count probes that were superseded by a later non-empty batch of the same resource
  const all = Array.from(byBatchId.values());
  const nonEmptyResources = new Set(
    all
      .filter((b) => b.resourceId && (b.totalCount === null || b.totalCount > 0))
      .map((b) => b.resourceId!)
  );
  return all.filter((b) => {
    if (b.totalCount === 0) {
      if (b.resourceId && nonEmptyResources.has(b.resourceId)) return false;
      if (all.some((other) => other.batchId !== b.batchId && (other.totalCount ?? 1) > 0)) {
        return false;
      }
    }
    return true;
  });
}

/** Synchronously reads the cached per-session workspace state if already hydrated. */
export function peekAgentSessionWorkspaceState(
  sessionId: string | null | undefined
): AgentSessionWorkspaceState | null {
  if (!sessionId || !sessionId.trim()) return null;
  return memoryWorkspaceBySession.get(sessionId.trim()) ?? null;
}

/** Clears test/fixture cache (primarily for unit tests). */
export function clearAgentSessionWorkspaceCache(): void {
  memoryWorkspaceBySession.clear();
}

export function resolveAgentSessionSourceLabel(opts: {
  batches: readonly AgentSessionBatchEntry[];
  activeFilter?: "all" | string;
  scenes?: readonly SceneRow[];
  effectiveResourceId?: string | null;
  fallbackSourceId?: string | null;
  sourceNames?: Readonly<Record<string, string>>;
  batchLabelsById?: Readonly<Record<string, string>>;
  fallbackLabel?: string | null;
}): string | null {
  const {
    batches,
    activeFilter = "all",
    scenes = [],
    effectiveResourceId,
    fallbackSourceId,
    sourceNames,
    batchLabelsById,
    fallbackLabel = null,
  } = opts;
  if (batches.length > 1) {
    if (activeFilter === "all") {
      return `${batches.length} 类数据源`;
    }
    const matched = batches.find((b) => b.batchId === activeFilter);
    if (matched) {
      return canonicalAgentDatasetShortName(
        matched.resourceId,
        matched.scenes,
        sourceNames,
        batchLabelsById?.[matched.batchId] ?? null
      );
    }
  }
  if (batches.length === 1) {
    const only = batches[0];
    return canonicalAgentDatasetShortName(
      only.resourceId,
      only.scenes,
      sourceNames,
      batchLabelsById?.[only.batchId] ?? null
    );
  }
  if (scenes.length > 0) {
    return canonicalAgentDatasetShortName(
      effectiveResourceId,
      scenes,
      sourceNames
    );
  }
  if (fallbackLabel && fallbackLabel.trim().length > 0) {
    return fallbackLabel;
  }
  return fallbackSourceId ? (sourceNames?.[fallbackSourceId] ?? null) : null;
}
