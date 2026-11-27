import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowLeft,
  CheckCircle2,
  Eye,
  FileUp,
  MapPin,
  RefreshCw,
  RotateCcw,
  Search,
  Trash2,
  X,
} from "lucide-react";

import type { AdminBoundary, AoiPreviewOk, Bbox, Json } from "@/lib/bridge";
import { DEFAULT_MAP_PROVIDER } from "@/lib/mapProviders";
import {
  availableCommonNisarSelections,
  combineNisarSubsetBoundaryFeatures,
  NisarSubsetPreflightGeneration,
  nisarSubsetWorkspaceReadiness,
  setNisarSubsetWorkspaceGeometry,
  toggleNisarSubsetWorkspaceItem,
  type NisarSubsetWorkspaceDraft,
  type NisarSubsetWorkspaceRow,
} from "@/lib/nisarSubsetWorkspace";
import { cn } from "@/lib/utils";
import { useFullTurnSpin } from "@/hooks/useLoadingPrimitives";
import { SubsetScissorsIcon } from "./desktop/SubsetScissorsIcon";
import {
  MAP_CHIP_BASE_CLASS,
  WorkbenchMap,
  type MapLayerKey,
  type WorkbenchDrawMode,
} from "./WorkbenchMap";
import { FrozenProductPreview } from "./FrozenProductPreview";
import { EMPTY_PREVIEW_STATE, type PreviewRuntimeState } from "@/lib/catalogPreview";
import { isAllowedByCsp, thumbnailUrlFor } from "@/shell/api/thumbnailLogic";
import { isBhoonidhiSsarCatalogProduct } from "@/lib/bhoonidhiSsarCatalog";
import {
  estimateNisarSubsetBatch,
  formatNisarSubsetEstimateSummary,
} from "@/lib/nisarSubsetEstimate";
import { NisarSbandSourceNotice } from "./NisarSbandSourceNotice";

type Geometry = Record<string, unknown>;

type SubsetPreviewOverlay = {
  identity: string;
  url?: string | null;
  bbox?: Bbox | null;
  geometry?: Geometry | null;
  label: string;
};

export type NisarSubsetWorkspaceAdminSearchResult =
  | { ok: true; results: AdminBoundary[] }
  | { ok: false; error: string };

export type NisarSubsetWorkspaceGeometryResult =
  | { ok: true; geometry: Geometry }
  | { ok: false; error: string };

export type NisarSubsetWorkspaceBoundaryImportResult =
  | { ok: true; preview: AoiPreviewOk }
  | { ok: false; error: string };

export type NisarSubsetWorkspacePreflight = {
  sourceBatchId: string;
  workbenchRevision: number;
  eligibleItems: Array<{
    sourceItemId: string;
    effectiveGeometry: Geometry;
    availableSelections: NisarSubsetWorkspaceRow["availableSelections"];
  }>;
  excludedItems: Array<{ sourceItemId: string; reason: "no_overlap" | "frequency_incompatible" | "polarization_incompatible" }>;
  commonSelections: NisarSubsetWorkspaceRow["availableSelections"];
};

export type NisarSubsetWorkspaceTaskContext = {
  taskId: string;
  status: string;
  revision: number;
  outputRoot: string;
  successor: boolean;
};

export type NisarSubsetWorkspaceProps = {
  taskContext?: NisarSubsetWorkspaceTaskContext;
  onChooseOutputDirectory?: () => Promise<void>;
  initialDraft: NisarSubsetWorkspaceDraft;
  rows: readonly NisarSubsetWorkspaceRow[];
  sourceBatchLabel: string;
  currentAoiName: string;
  mapBbox?: Bbox | null;
  effectiveGeometries?: readonly Geometry[];
  onClose: () => void;
  onPreviewItem?: (itemId: string) => void;
  reviewBusy?: boolean;
  reviewError?: string | null;
  onPreflight?: (draft: NisarSubsetWorkspaceDraft) => Promise<NisarSubsetWorkspacePreflight>;
  onReview: (draft: NisarSubsetWorkspaceDraft, checkedRows: NisarSubsetWorkspaceRow[]) => Promise<void> | void;
  onSearchAdministrativeAoi?: (
    query: string,
  ) => Promise<NisarSubsetWorkspaceAdminSearchResult>;
  onResolveAdministrativeAoi?: (
    boundary: AdminBoundary,
  ) => Promise<NisarSubsetWorkspaceGeometryResult>;
  onImportBoundary?: () => Promise<NisarSubsetWorkspaceBoundaryImportResult>;
};

function geometryFromBbox(bbox: Bbox): Geometry {
  return {
    type: "Polygon",
    coordinates: [[
      [bbox.west, bbox.south],
      [bbox.east, bbox.south],
      [bbox.east, bbox.north],
      [bbox.west, bbox.north],
      [bbox.west, bbox.south],
    ]],
  };
}

function geometryFromBoundary(boundary: AdminBoundary): Geometry | null {
  const value = boundary.geometry ?? boundary.geojson;
  return value && typeof value === "object" && !Array.isArray(value)
    ? structuredClone(value as Geometry)
    : null;
}

function itemLabel(row: NisarSubsetWorkspaceRow): string {
  return row.product.product_id || row.product.scene_id || row.itemId;
}

function extractGeometryBbox(geo: Geometry | null | undefined): Bbox | null {
  if (!geo || typeof geo !== "object") return null;
  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;
  const visitCoords = (coords: unknown) => {
    if (!Array.isArray(coords)) return;
    if (coords.length >= 2 && typeof coords[0] === "number" && typeof coords[1] === "number") {
      const lng = coords[0];
      const lat = coords[1];
      if (Number.isFinite(lng) && Number.isFinite(lat)) {
        if (lng < west) west = lng;
        if (lng > east) east = lng;
        if (lat < south) south = lat;
        if (lat > north) north = lat;
      }
      return;
    }
    for (const child of coords) visitCoords(child);
  };
  const visitGeometry = (node: unknown) => {
    if (!node || typeof node !== "object") return;
    const record = node as { coordinates?: unknown; geometries?: unknown[]; canonical_geojson?: unknown };
    if (record.canonical_geojson && typeof record.canonical_geojson === "object") {
      visitGeometry(record.canonical_geojson);
      return;
    }
    if (record.coordinates !== undefined) visitCoords(record.coordinates);
    if (Array.isArray(record.geometries)) {
      for (const child of record.geometries) visitGeometry(child);
    }
  };
  visitGeometry(geo);
  if (!Number.isFinite(west) || !Number.isFinite(south) || !Number.isFinite(east) || !Number.isFinite(north)) {
    return null;
  }
  return { west, south, east, north, crs: "EPSG:4326" };
}

function extractFootprintBbox(row: NisarSubsetWorkspaceRow): Bbox | null {
  const explicit = row.product.footprint_bbox;
  if (
    explicit &&
    Number.isFinite(explicit.west) &&
    Number.isFinite(explicit.south) &&
    Number.isFinite(explicit.east) &&
    Number.isFinite(explicit.north)
  ) {
    return explicit;
  }
  return extractGeometryBbox(row.product.footprint_geojson as Geometry | null | undefined);
}

function unionFootprintBboxes(rows: readonly NisarSubsetWorkspaceRow[]): Bbox | null {
  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;
  for (const row of rows) {
    const box = extractFootprintBbox(row);
    if (!box) continue;
    if (box.west < west) west = box.west;
    if (box.south < south) south = box.south;
    if (box.east > east) east = box.east;
    if (box.north > north) north = box.north;
  }
  if (!Number.isFinite(west) || !Number.isFinite(south) || !Number.isFinite(east) || !Number.isFinite(north)) {
    return null;
  }
  return { west, south, east, north, crs: "EPSG:4326" };
}

function displayError(error: unknown): string {
  return error instanceof Error && error.message.trim()
    ? error.message
    : "操作失败，请重试。";
}

export function NisarSubsetWorkspace({
  taskContext,
  onChooseOutputDirectory,
  initialDraft,
  rows,
  sourceBatchLabel,
  currentAoiName,
  mapBbox,
  effectiveGeometries = [],
  onClose,
  onPreviewItem,
  reviewBusy = false,
  reviewError = null,
  onPreflight,
  onReview,
  onSearchAdministrativeAoi,
  onResolveAdministrativeAoi,
  onImportBoundary,
}: NisarSubsetWorkspaceProps) {
  const initialGeometryRef = useRef<Geometry | null>(
    initialDraft.requestedGeometry
      ? structuredClone(initialDraft.requestedGeometry)
      : null,
  );
  const [frozenRows, setFrozenRows] = useState<NisarSubsetWorkspaceRow[]>(() => structuredClone([...rows]));
  const [draft, setDraft] = useState<NisarSubsetWorkspaceDraft>(() => (
    structuredClone(initialDraft)
  ));
  const [query, setQuery] = useState("");
  const [previewItemId, setPreviewItemId] = useState<string | null>(null);
  const [resolvedPreviewState, setResolvedPreviewState] = useState<PreviewRuntimeState>(EMPTY_PREVIEW_STATE);
  const previewRow = frozenRows.find((row) => row.itemId === previewItemId);
  const previewOverlay = useMemo<SubsetPreviewOverlay | null>(() => {
    if (!previewRow) return null;
    const bbox = extractFootprintBbox(previewRow);
    const geometry =
      (previewRow.product.footprint_geojson as Geometry | null) ??
      (bbox ? geometryFromBbox(bbox) : null);
    const resolvedUrl =
      resolvedPreviewState.product_key === previewRow.itemId &&
      resolvedPreviewState.status === "available" &&
      resolvedPreviewState.url
        ? resolvedPreviewState.url
        : null;
    const rawThumb = thumbnailUrlFor({ sar_product: previewRow.product } as unknown as Record<string, unknown>);
    const fallbackUrl = rawThumb && isAllowedByCsp(rawThumb) ? rawThumb : null;
    return {
      identity: previewRow.itemId,
      url: resolvedUrl ?? fallbackUrl,
      bbox,
      geometry,
      label: itemLabel(previewRow),
    };
  }, [previewRow, resolvedPreviewState]);
  const [adminQuery, setAdminQuery] = useState("");
  const [candidateFilter, setCandidateFilter] = useState<"all" | "eligible" | "excluded" | "selected">("all");
  const automaticPreflightTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [mapLayerKey, setMapLayerKey] = useState<MapLayerKey>(DEFAULT_MAP_PROVIDER.key);
  const [drawMode, setDrawMode] = useState<WorkbenchDrawMode>("rect");
  const [drawActive, setDrawActive] = useState<boolean>(false);
  const [adminOpen, setAdminOpen] = useState(false);
  const [adminBusy, setAdminBusy] = useState(false);
  const [adminResults, setAdminResults] = useState<AdminBoundary[]>([]);
  const [boundaryBusy, setBoundaryBusy] = useState(false);
  const [boundaryPreview, setBoundaryPreview] = useState<AoiPreviewOk | null>(null);
  const [selectedBoundaryFeatureIds, setSelectedBoundaryFeatureIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [preflightBusy, setPreflightBusy] = useState(false);
  const { spinClass: preflightSpinClass } = useFullTurnSpin(preflightBusy);
  const [preflightInputEpoch, setPreflightInputEpoch] = useState(0);
  const [preflightEffectiveGeometries, setPreflightEffectiveGeometries] = useState<Geometry[]>([]);
  const preflightGenerationRef = useRef<NisarSubsetPreflightGeneration | null>(null);
  if (preflightGenerationRef.current === null) {
    preflightGenerationRef.current = new NisarSubsetPreflightGeneration();
  }
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (reviewBusy) {
        event.preventDefault();
        return;
      }
      if (boundaryPreview) {
        event.preventDefault();
        setBoundaryPreview(null);
        return;
      }
      onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [boundaryPreview, onClose, reviewBusy]);

  const selectedIds = useMemo(() => new Set(draft.selectedItemIds), [draft.selectedItemIds]);
  const selected = useMemo(() => frozenRows.filter((row) => selectedIds.has(row.itemId)), [selectedIds, frozenRows]);
  const requestedBbox = useMemo(
    () => extractGeometryBbox(draft.requestedGeometry),
    [draft.requestedGeometry],
  );
  const candidateUnionBbox = useMemo(
    () => unionFootprintBboxes(selected.length > 0 ? selected : frozenRows),
    [selected, frozenRows],
  );
  const effectiveMapBbox = requestedBbox ?? mapBbox ?? candidateUnionBbox ?? null;
  const activeEffectiveGeometries = onPreflight ? preflightEffectiveGeometries : effectiveGeometries;
  const rowsById = useMemo(() => {
    const index = new Map<string, NisarSubsetWorkspaceRow>();
    for (const row of frozenRows) if (!index.has(row.itemId)) index.set(row.itemId, row);
    return index;
  }, [frozenRows]);
  const hasSbandSource = useMemo(
    () => frozenRows.some((row) => isBhoonidhiSsarCatalogProduct(row.product)),
    [frozenRows],
  );
  const executableSelected = useMemo(
    () => selected.filter((row) => row.overlapState === "intersects"),
    [selected],
  );
  const commonSelections = useMemo(
    () => availableCommonNisarSelections(executableSelected),
    [executableSelected],
  );
  const readiness = useMemo(
    () => nisarSubsetWorkspaceReadiness(draft, frozenRows),
    [draft, frozenRows],
  );
  const selectedChoice = commonSelections.find(
    (choice) => choice.frequency === draft.selectedFrequency,
  ) ?? null;
  const liveEstimateSummary = useMemo(() => {
    const targetRows = executableSelected.length > 0 ? executableSelected : selected.length > 0 ? selected : frozenRows;
    const estScenes = targetRows.map((row) => {
      const prod = row.product as unknown as Record<string, unknown>;
      const rawBytes =
        typeof prod.file_size_remote === "number"
          ? prod.file_size_remote
          : typeof prod.size_bytes === "number"
            ? prod.size_bytes
            : typeof prod.file_size_bytes === "number"
              ? prod.file_size_bytes
              : null;
      const pols = Array.isArray(prod.polarizations)
        ? prod.polarizations.filter((p): p is string => typeof p === "string")
        : [];
      return {
        product_key: row.itemId,
        footprint_bbox: extractFootprintBbox(row),
        file_size_remote: rawBytes,
        polarization: pols.length > 0 ? pols.join("+") : null,
      };
    });
    const est = estimateNisarSubsetBatch({
      scenes: estScenes,
      sceneCount: estScenes.length || 1,
      aoiBbox: requestedBbox ?? mapBbox ?? null,
      aoiGeometry: draft.requestedGeometry,
      polarizations: draft.selectedPolarizations,
      availablePolarizations: selectedChoice?.polarizations,
    });
    return formatNisarSubsetEstimateSummary(est);
  }, [
    executableSelected,
    selected,
    frozenRows,
    requestedBbox,
    mapBbox,
    draft.requestedGeometry,
    draft.selectedPolarizations,
    selectedChoice?.polarizations,
  ]);
  const footprintGeometries = selected.flatMap((row) => {
    const value = row.product.footprint_geojson;
    return value && typeof value === "object" && !Array.isArray(value)
      ? [value as Geometry]
      : [];
  });
  const conflictingLabels = readiness.conflictingItemIds.map((itemId) => {
    const row = rowsById.get(itemId);
    return row ? itemLabel(row) : itemId;
  });

  function invalidatePreflight() {
    preflightGenerationRef.current?.invalidate();
    if (automaticPreflightTimer.current !== null) clearTimeout(automaticPreflightTimer.current);
    setPreflightBusy(false);
    setFrozenRows((current) => current.map((row) => ({ ...row, overlapState: "unchecked" })));
    setPreflightEffectiveGeometries([]);
  }

  function updateGeometry(geometry: Geometry | null) {
    if (taskContext) return;
    invalidatePreflight();
    setPreflightInputEpoch((epoch) => epoch + 1);
    setDraft((current) => ({
      ...setNisarSubsetWorkspaceGeometry(current, geometry),
      selectedFrequency: null,
      selectedPolarizations: [],
    }));
    setError(null);
  }

  async function preflight() {
    if (!onPreflight || preflightBusy || !draft.requestedGeometry || draft.selectedItemIds.length === 0) return;
    if (automaticPreflightTimer.current !== null) clearTimeout(automaticPreflightTimer.current);
    const requestGeneration = preflightGenerationRef.current!.begin();
    const requestDraft = structuredClone(draft);
    setPreflightBusy(true);
    setError(null);
    try {
      const result = await onPreflight(requestDraft);
      if (!preflightGenerationRef.current?.isCurrent(requestGeneration)) return;
      const eligibleById = new Map(result.eligibleItems.map((item) => [item.sourceItemId, item]));
      const excludedById = new Map(result.excludedItems.map((item) => [item.sourceItemId, item]));
      const selectedIds = new Set(requestDraft.selectedItemIds);
      if (
        result.sourceBatchId !== requestDraft.sourceBatchId
        || eligibleById.size !== result.eligibleItems.length
        || excludedById.size !== result.excludedItems.length
        || eligibleById.size + excludedById.size !== selectedIds.size
        || [...eligibleById.keys()].some((itemId) => !selectedIds.has(itemId) || excludedById.has(itemId))
        || [...selectedIds].some((itemId) => !eligibleById.has(itemId) && !excludedById.has(itemId))
      ) throw new Error("权威检查结果与当前裁剪草稿不一致，请重试。");
      setFrozenRows((current) => current.map((row) => {
        if (!selectedIds.has(row.itemId)) return { ...row, overlapState: "unchecked" };
        const eligible = eligibleById.get(row.itemId);
        if (eligible) return {
          ...row,
          overlapState: "intersects",
          availableSelections: structuredClone(eligible.availableSelections),
        };
        const reason = excludedById.get(row.itemId)?.reason;
        return reason === "frequency_incompatible" || reason === "polarization_incompatible"
          ? { ...row, overlapState: "incompatible", exclusionReason: reason }
          : { ...row, overlapState: "no_overlap", exclusionReason: undefined };
      }));
      setPreflightEffectiveGeometries(result.eligibleItems.map((item) => structuredClone(item.effectiveGeometry)));
      setDraft((current) => {
        if (taskContext) return current;
        const choice = result.commonSelections.find((entry) => entry.frequency === current.selectedFrequency);
        const stillAvailable = choice && current.selectedPolarizations.length > 0
          && current.selectedPolarizations.every((polarization) => choice.polarizations.includes(polarization));
        return stillAvailable ? current : { ...current, selectedFrequency: null, selectedPolarizations: [] };
      });
    } catch (caught) {
      if (!preflightGenerationRef.current?.isCurrent(requestGeneration)) return;
      invalidatePreflight();
      setError(displayError(caught));
    } finally {
      if (preflightGenerationRef.current?.isCurrent(requestGeneration)) {
        setPreflightBusy(false);
      }
    }
  }

  // Only spatial inputs trigger a request. Selecting frequency/polarizations must
  // not restart preflight or erase the user's scientific selection.
  const preflightInputKey = JSON.stringify([draft.sourceBatchId, draft.requestedGeometry, draft.selectedItemIds]);
  const runPreflightRef = useRef(preflight);
  runPreflightRef.current = preflight;
  useEffect(() => {
    automaticPreflightTimer.current = setTimeout(() => void runPreflightRef.current(), 200);
    return () => {
      if (automaticPreflightTimer.current !== null) clearTimeout(automaticPreflightTimer.current);
      preflightGenerationRef.current?.invalidate();
    };
  }, [preflightInputKey, preflightInputEpoch]);

  const visibleRows = frozenRows.filter((row) => {
    if (!itemLabel(row).toLowerCase().includes(query.trim().toLowerCase())) return false;
    if (candidateFilter === "selected") return selectedIds.has(row.itemId);
    if (candidateFilter === "eligible") return row.subsetEligible && row.overlapState === "intersects";
    if (candidateFilter === "excluded") return !row.subsetEligible || row.overlapState === "no_overlap" || row.overlapState === "incompatible";
    return true;
  });

  function selectFrequency(frequency: "A" | "B") {
    if (taskContext) return;
    setDraft((current) => ({
      ...current,
      selectedFrequency: frequency,
      selectedPolarizations: [],
    }));
  }

  function togglePolarization(polarization: string) {
    if (taskContext) return;
    setDraft((current) => {
      const values = new Set(current.selectedPolarizations);
      if (values.has(polarization)) values.delete(polarization);
      else values.add(polarization);
      return {
        ...current,
        selectedPolarizations: [...values].sort((left, right) => left.localeCompare(right)),
      };
    });
  }

  async function searchAdministrativeAoi() {
    if (!onSearchAdministrativeAoi || !adminQuery.trim()) return;
    setAdminBusy(true);
    setError(null);
    setAdminResults([]);
    try {
      const result = await onSearchAdministrativeAoi(adminQuery.trim());
      if (result.ok) setAdminResults(result.results);
      else setError(result.error);
    } catch (caught) {
      setError(displayError(caught));
    } finally {
      setAdminBusy(false);
    }
  }

  async function useAdministrativeAoi(boundary: AdminBoundary) {
    setBoundaryBusy(true);
    setError(null);
    try {
      const embedded = geometryFromBoundary(boundary);
      const result = embedded
        ? { ok: true as const, geometry: embedded }
        : onResolveAdministrativeAoi
          ? await onResolveAdministrativeAoi(boundary)
          : { ok: false as const, error: "当前行政区尚未解析出真实边界。" };
      if (result.ok) {
        updateGeometry(result.geometry);
        setAdminOpen(false);
      } else {
        setError(result.error);
      }
    } catch (caught) {
      setError(displayError(caught));
    } finally {
      setBoundaryBusy(false);
    }
  }

  async function importBoundary() {
    if (!onImportBoundary) return;
    setBoundaryBusy(true);
    setError(null);
    try {
      const result = await onImportBoundary();
      if (result.ok) {
        setBoundaryPreview(structuredClone(result.preview));
        setSelectedBoundaryFeatureIds(new Set());
      } else {
        setError(result.error);
      }
    } catch (caught) {
      setError(displayError(caught));
    } finally {
      setBoundaryBusy(false);
    }
  }

  function toggleBoundaryFeature(featureId: string) {
    setSelectedBoundaryFeatureIds((current) => {
      const next = new Set(current);
      if (next.has(featureId)) next.delete(featureId);
      else next.add(featureId);
      return next;
    });
    setError(null);
  }

  function applyBoundaryFeatures() {
    if (!boundaryPreview) return;
    const result = combineNisarSubsetBoundaryFeatures(
      boundaryPreview,
      selectedBoundaryFeatureIds,
    );
    if (!result.ok) {
      setError(result.error);
      return;
    }
    updateGeometry(result.geometry);
    setBoundaryPreview(null);
    setSelectedBoundaryFeatureIds(new Set());
  }

  const allSelectedChecked =
    selected.length > 0 && selected.every((row) => row.overlapState !== "unchecked");

  const subsetLegendControls = (
    <ul
      role="list"
      aria-label="裁剪地图图层"
      className={cn(
        MAP_CHIP_BASE_CLASS,
        "inline-flex flex-wrap items-center gap-2.5 py-1 select-none",
      )}
    >
      <li className="inline-flex items-center gap-1">
        <span
          className="h-2 w-2 rounded-xs border border-dashed border-slate-500 bg-slate-500/15"
          aria-hidden="true"
        />
        影像覆盖
      </li>
      {previewOverlay && (
        <li className="inline-flex items-center gap-1">
          <span
            className="h-2 w-2 rounded-xs border border-amber-600 bg-amber-500/20"
            aria-hidden="true"
          />
          当前预览影像框
        </li>
      )}
      <li className="inline-flex items-center gap-1">
        <span
          className="h-2 w-2 rounded-xs border border-amber-500 bg-amber-400/25"
          aria-hidden="true"
        />
        请求范围
      </li>
      {activeEffectiveGeometries.length > 0 && (
        <li className="inline-flex items-center gap-1">
          <span
            className="h-2 w-2 rounded-xs border border-emerald-600 bg-emerald-500/25"
            aria-hidden="true"
          />
          实际相交范围
        </li>
      )}
    </ul>
  );

  return (
    <div className="fixed inset-0 z-[1200] flex bg-black/25">
      <section
        role="dialog"
        aria-modal="true"
        aria-label="NISAR RSLC AOI 裁剪下载"
        className="flex min-h-0 w-full flex-col bg-sp-canvas text-sp-text"
      >
        {/* 顶部 42px 紧凑工作台栏（与 ManualWorkbench 视觉与高度完全一致） */}
        <header className="flex min-h-[42px] shrink-0 items-center gap-2.5 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3.5 py-1.5 select-none">
          <button
            type="button"
            onClick={onClose}
            disabled={reviewBusy}
            className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-text/30 cursor-pointer disabled:cursor-not-allowed disabled:opacity-50"
          >
            <ArrowLeft className="h-3.5 w-3.5 text-sp-text-secondary" />
            返回
          </button>

          <span className="h-3.5 w-px bg-sp-line shrink-0" aria-hidden="true" />

          <SubsetScissorsIcon className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />

          <div className="min-w-0 flex flex-1 items-center gap-2">
            <h2 className="text-[13px] font-semibold tracking-tight text-sp-text truncate">
              {taskContext
                ? taskContext.successor
                  ? "NISAR RSLC AOI 裁剪 · 新建后继任务"
                  : "NISAR RSLC AOI 裁剪 · 追加数据"
                : "NISAR RSLC AOI 裁剪下载"}
            </h2>
            <span className="shrink-0 rounded-sp-sm border border-sp-line bg-sp-surface px-1.5 py-0.5 font-mono text-[11px] text-sp-text-secondary">
              {hasSbandSource ? "S-Band · Bhoonidhi" : "L-Band · Earthdata"}
            </span>
            <span
              className="hidden lg:inline truncate text-[11px] font-mono text-sp-text-muted before:mr-2 before:content-['·']"
              title={sourceBatchLabel}
            >
              来源批次 {sourceBatchLabel}
            </span>
          </div>

          <span className="inline-flex items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-0.5 font-mono text-[11px] font-medium text-sp-text tabular-nums shrink-0">
            已选 {draft.selectedItemIds.length} 景
          </span>
          <span
            className={cn(
              "inline-flex items-center rounded-sp-sm border px-2 py-0.5 text-[11px] font-medium shrink-0",
              draft.requestedGeometry
                ? "border-sp-line bg-sp-surface text-sp-text"
                : "border-sp-warning-text/30 bg-sp-warning-bg text-sp-warning-text",
            )}
          >
            {draft.requestedGeometry ? "已选择 AOI" : "尚未选择 AOI"}
          </span>

          <button
            type="button"
            aria-label="关闭裁剪工作区"
            onClick={onClose}
            disabled={reviewBusy}
            className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-text/30 cursor-pointer disabled:cursor-not-allowed disabled:opacity-40"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {hasSbandSource && <NisarSbandSourceNotice />}

        {/* 主体三栏工作台：与主界面 ManualWorkbench 保持统一分栏比例与无框地图画布 */}
        <div className="grid min-h-0 flex-1 overflow-y-auto grid-cols-1 md:grid-cols-[minmax(210px,0.8fr)_minmax(280px,1.6fr)_minmax(230px,0.9fr)] lg:grid-cols-[minmax(270px,310px)_minmax(360px,1fr)_minmax(290px,330px)] md:overflow-hidden">
          {/* 左栏：候选影像筛选与勾选 */}
          <aside className="flex min-h-[260px] min-w-0 flex-col border-b border-sp-line bg-sp-surface md:min-h-0 md:border-b-0 md:border-r">
            <div className="flex h-[42px] shrink-0 items-center justify-between border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3.5 select-none">
              <h3 className="text-[13px] font-semibold tracking-tight text-sp-text">选择影像</h3>
              <span className="font-mono text-[11px] text-sp-text-muted tabular-nums">
                显示 {visibleRows.length} / {frozenRows.length} 景
              </span>
            </div>

            <div className="border-b border-sp-line bg-sp-surface px-3 py-2 space-y-2">
              <label className="relative block">
                <span className="sr-only">搜索影像</span>
                <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-sp-text-muted" />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="搜索产品编号或场景标识…"
                  className="h-7 w-full rounded-sp-sm border border-sp-line bg-sp-surface-muted/50 pl-8 pr-2.5 text-xs text-sp-text placeholder:text-sp-text-muted focus:border-sp-text/45 focus:bg-sp-surface focus:outline-none transition-colors"
                />
              </label>

              <div
                className="grid grid-cols-4 gap-0.5 rounded-sp-sm border border-sp-line bg-sp-surface-muted p-0.5"
                role="group"
                aria-label="筛选裁剪候选"
              >
                {([["all", "全部"], ["eligible", "可执行"], ["excluded", "已排除"], ["selected", "已选择"]] as const).map(([value, label]) => (
                  <button
                    key={value}
                    type="button"
                    aria-pressed={candidateFilter === value}
                    onClick={() => setCandidateFilter(value)}
                    className={cn(
                      "rounded-xs py-1 text-[11px] font-medium transition-colors cursor-pointer",
                      candidateFilter === value
                        ? "bg-sp-surface font-semibold text-sp-text shadow-xs"
                        : "text-sp-text-secondary hover:text-sp-text"
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto divide-y divide-sp-line">
              {visibleRows.length === 0 && (
                <div className="px-4 py-8 text-center text-xs text-sp-text-muted">
                  没有符合当前筛选的影像。
                </div>
              )}
              {visibleRows.map((row) => {
                const label = itemLabel(row);
                const checked = selectedIds.has(row.itemId);
                const isIntersects = row.subsetEligible && row.overlapState === "intersects";
                const isExcluded =
                  !row.subsetEligible ||
                  row.overlapState === "no_overlap" ||
                  row.overlapState === "incompatible";

                return (
                  <div
                    key={row.itemId}
                    className={cn(
                      "px-3 py-2.5 transition-colors duration-fast",
                      checked
                        ? "bg-sp-selected/65"
                        : "bg-sp-surface hover:bg-sp-hover/60"
                    )}
                  >
                    <label className="flex min-w-0 items-start gap-2.5 cursor-pointer">
                      <input
                        type="checkbox"
                        aria-label={`${label} 选择影像`}
                        checked={checked}
                        disabled={!row.subsetEligible || reviewBusy}
                        onChange={() => {
                          invalidatePreflight();
                          setPreflightInputEpoch((epoch) => epoch + 1);
                          setDraft((current) => ({
                            ...toggleNisarSubsetWorkspaceItem(current, frozenRows, row.itemId),
                            selectedFrequency: taskContext ? current.selectedFrequency : null,
                            selectedPolarizations: taskContext ? current.selectedPolarizations : [],
                          }));
                        }}
                        className="mt-0.5 h-3.5 w-3.5 rounded-xs border-sp-line accent-neutral-900 dark:accent-neutral-100"
                      />
                      <span className="min-w-0 flex-1 space-y-1">
                        <span className="block break-all font-mono text-xs font-medium text-sp-text leading-snug">
                          {label}
                        </span>
                        <span className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] text-sp-text-muted">
                          <span>{row.product.acquisition_start ?? "时间未知"}</span>
                          <span>·</span>
                          <span className="font-mono">{row.product.provider}</span>
                        </span>
                        <span className="inline-block rounded-xs border border-sp-line bg-sp-surface-muted/70 px-1.5 py-0.5 font-mono text-[10px] text-sp-text-secondary">
                          {row.availableSelections.length > 0
                            ? row.availableSelections.map((choice) => `${choice.frequency}: ${choice.polarizations.join(" / ")}`).join(" · ")
                            : "频率与极化待检查"}
                        </span>
                      </span>
                    </label>
                    <div className="mt-2 flex items-center justify-between gap-2 pl-6 text-[11px]">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-xs px-1.5 py-0.5 font-medium",
                          isIntersects
                            ? "bg-sp-success-bg text-sp-success-text"
                            : isExcluded
                              ? "bg-sp-warning-bg text-sp-warning-text"
                              : "bg-sp-surface-muted text-sp-text-secondary"
                        )}
                      >
                        {!row.subsetEligible
                          ? row.blockedReason ?? "不可裁剪"
                          : row.overlapState === "incompatible"
                            ? row.exclusionReason === "frequency_incompatible"
                              ? "不支持原任务频率，已排除"
                              : "不支持原任务极化，已排除"
                            : row.overlapState === "no_overlap"
                              ? "与 AOI 无交集，将排除"
                              : row.overlapState === "intersects"
                                ? "已确认相交"
                                : "相交范围待检查"}
                      </span>
                      <button
                        type="button"
                        aria-label={`预览影像 ${row.itemId}`}
                        onClick={() => {
                          setPreviewItemId(row.itemId);
                          onPreviewItem?.(row.itemId);
                        }}
                        className={cn(
                          "inline-flex shrink-0 items-center gap-1 rounded-sp-sm border px-2 py-0.5 text-[11px] font-medium transition-colors cursor-pointer",
                          previewItemId === row.itemId
                            ? "border-sp-text/40 bg-sp-selected text-sp-text"
                            : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                        )}
                      >
                        <Eye className="h-3 w-3" />
                        预览
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </aside>

          {/* 中栏：复用主工作台原生 WorkbenchMap（统一工具条、底图/注记切换、底部坐标比例栏） */}
          <main className="flex min-h-[360px] min-w-0 flex-col border-b border-sp-line bg-sp-canvas md:min-h-0 md:border-b-0 md:border-r">
            {taskContext ? (
              <div className="flex min-h-[42px] shrink-0 items-center justify-between gap-3 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3.5 py-1.5 text-xs">
                <span className="font-semibold text-sp-text">任务冻结 AOI（Task Frozen AOI）</span>
                <span className="truncate text-sp-text-muted">沿用原任务的裁剪范围、频率和极化。</span>
              </div>
            ) : (
              <div className="flex min-h-[42px] shrink-0 flex-wrap items-center gap-1.5 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3 py-1.5">
                <button
                  type="button"
                  disabled={reviewBusy}
                  onClick={() => updateGeometry(initialGeometryRef.current)}
                  className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 transition-colors cursor-pointer"
                >
                  使用当前 AOI
                </button>
                <button
                  type="button"
                  disabled={reviewBusy}
                  onClick={() => setAdminOpen((open) => !open)}
                  className={cn(
                    "inline-flex h-7 items-center gap-1 rounded-sp-sm border px-2.5 text-xs font-medium transition-colors cursor-pointer disabled:opacity-50",
                    adminOpen
                      ? "border-sp-text/40 bg-sp-selected text-sp-text"
                      : "border-sp-line bg-sp-surface text-sp-text hover:bg-sp-hover"
                  )}
                >
                  <MapPin className="h-3 w-3 text-sp-text-secondary" />
                  选择行政区
                </button>
                <button
                  type="button"
                  disabled={!onImportBoundary || boundaryBusy || reviewBusy}
                  onClick={() => void importBoundary()}
                  className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 transition-colors cursor-pointer"
                >
                  <FileUp className="h-3 w-3 text-sp-text-secondary" />
                  导入边界
                </button>

                <span className="h-3.5 w-px bg-sp-line mx-0.5" aria-hidden="true" />

                <button
                  type="button"
                  disabled={reviewBusy}
                  onClick={() => updateGeometry(initialGeometryRef.current)}
                  className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text disabled:opacity-50 transition-colors cursor-pointer"
                >
                  <RotateCcw className="h-3 w-3" />
                  重置范围
                </button>
                <button
                  type="button"
                  disabled={reviewBusy}
                  onClick={() => updateGeometry(null)}
                  className="inline-flex h-7 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2 text-xs text-sp-text-secondary hover:bg-sp-danger-bg hover:text-sp-danger-text disabled:opacity-50 transition-colors cursor-pointer"
                >
                  <Trash2 className="h-3 w-3" />
                  清空范围
                </button>

                <span
                  className="ml-auto truncate text-[11px] text-sp-text-muted"
                  title={currentAoiName}
                >
                  打开时 AOI：{currentAoiName}
                </span>
              </div>
            )}

            {adminOpen && (
              <div className="border-b border-sp-line bg-sp-surface px-3 py-2 space-y-2">
                <div className="flex gap-2">
                  <label className="min-w-0 flex-1">
                    <span className="sr-only">行政区关键词</span>
                    <input
                      aria-label="行政区关键词"
                      placeholder="输入省/市/县名称检索裁剪边界…"
                      value={adminQuery}
                      onChange={(event) => setAdminQuery(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") void searchAdministrativeAoi();
                      }}
                      className="h-7 w-full rounded-sp-sm border border-sp-line bg-sp-surface-muted/40 px-2.5 text-xs text-sp-text placeholder:text-sp-text-muted focus:border-sp-text/45 focus:bg-sp-surface focus:outline-none"
                    />
                  </label>
                  <button
                    type="button"
                    disabled={adminBusy || !adminQuery.trim() || !onSearchAdministrativeAoi}
                    onClick={() => void searchAdministrativeAoi()}
                    className="eo-settings-btn-primary h-7 rounded-sp-sm px-3 text-xs font-semibold disabled:opacity-50 cursor-pointer"
                  >
                    {adminBusy ? "搜索中…" : "搜索行政区"}
                  </button>
                </div>
                {adminResults.length > 0 && (
                  <div className="grid max-h-28 gap-1.5 overflow-y-auto sm:grid-cols-2">
                    {adminResults.map((boundary, index) => {
                      const label = boundary.display_name ?? boundary.label;
                      return (
                        <button
                          key={`${boundary.provider ?? "provider"}:${boundary.provider_object_id ?? index}`}
                          type="button"
                          aria-label={`使用行政区：${label}`}
                          disabled={boundaryBusy}
                          onClick={() => void useAdministrativeAoi(boundary)}
                          className="rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-1.5 text-left text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                        >
                          {label}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            )}

            <div className="relative min-h-[300px] flex-1 overflow-hidden bg-sp-canvas">
              <WorkbenchMap
                declutter
                chrome="panel"
                hideDefaultLegend
                fitScope="aoi"
                lockViewportOnSceneChange={true}
                bbox={effectiveMapBbox}
                aoiBbox={requestedBbox}
                aoiGeometry={(draft.requestedGeometry ?? null) as Json | null}
                aoiName={draft.requestedGeometry ? "裁剪请求范围" : undefined}
                layerKey={mapLayerKey}
                onLayerChange={setMapLayerKey}
                drawMode={drawMode}
                drawActive={!taskContext && !reviewBusy && drawActive}
                drawDisabled={Boolean(taskContext) || reviewBusy}
                onDrawModeChange={(mode) => {
                  if (!taskContext && !reviewBusy && (mode === "rect" || mode === "polygon")) {
                    setDrawMode(mode);
                  }
                }}
                onDrawActiveChange={(next) => {
                  if (!taskContext && !reviewBusy) {
                    setDrawActive(next);
                  }
                }}
                onClearLayers={() => {
                  if (!taskContext && !reviewBusy) {
                    updateGeometry(null);
                  }
                }}
                onRectDraw={(drawn) => {
                  if (taskContext || reviewBusy) return;
                  updateGeometry(geometryFromBbox(drawn));
                  setDrawActive(false);
                }}
                onPolygonDraw={(ring) => {
                  if (taskContext || reviewBusy) return;
                  updateGeometry({ type: "Polygon", coordinates: [ring] });
                  setDrawActive(false);
                }}
                onPointDraw={() => undefined}
                subsetFootprintGeometries={footprintGeometries as readonly Json[]}
                subsetPreviewGeometry={(previewOverlay?.geometry ?? null) as Json | null}
                subsetPreviewImageBbox={previewOverlay?.bbox ?? null}
                subsetPreviewImageUrl={previewOverlay?.url ?? null}
                subsetEffectiveGeometries={activeEffectiveGeometries as readonly Json[]}
                topControls={subsetLegendControls}
              />
              {previewRow && (
                <div className="pointer-events-none absolute inset-x-0 top-0 bottom-10 z-[530]">
                  <FrozenProductPreview
                    batchId={draft.sourceBatchId}
                    itemId={previewRow.itemId}
                    productLabel={itemLabel(previewRow)}
                    onClose={() => {
                      setPreviewItemId(null);
                      setResolvedPreviewState(EMPTY_PREVIEW_STATE);
                    }}
                    onPreviewChange={setResolvedPreviewState}
                    compactOverlay
                  />
                </div>
              )}
            </div>
          </main>

          {/* 右栏：科学裁剪参数配置与提交 */}
          <aside className="flex min-h-[300px] min-w-0 flex-col bg-sp-surface md:min-h-0">
            <div className="flex h-[42px] shrink-0 items-center justify-between border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3.5 select-none">
              <h3 className="text-[13px] font-semibold tracking-tight text-sp-text">裁剪设置</h3>
              <span className="text-[11px] text-sp-text-muted">按真实足迹计算</span>
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto divide-y divide-sp-line">
              {taskContext && (
                <div className="space-y-1.5 px-3.5 py-3 text-xs">
                  <p className="break-all font-mono text-sp-text">来源任务：{taskContext.taskId}</p>
                  <p className="font-mono font-semibold text-sp-text">
                    Frequency {draft.selectedFrequency} · {draft.selectedPolarizations.join(" / ")}
                  </p>
                  <p className="break-all text-sp-text-secondary">
                    输出目录：{taskContext.outputRoot || "请选择输出目录"}
                  </p>
                  {taskContext.successor && onChooseOutputDirectory && (
                    <button
                      type="button"
                      className="rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-1 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
                      onClick={() => void onChooseOutputDirectory()}
                    >
                      选择输出目录
                    </button>
                  )}
                  <p className="text-sp-text-muted">
                    {taskContext.successor
                      ? "将创建新的裁剪任务，原任务保持不变。"
                      : "符合原任务裁剪参数的新影像将追加到当前任务。"}
                  </p>
                </div>
              )}

              {/* 步骤 1：相交与能力预检 */}
              <div className="space-y-2.5 px-3.5 py-3">
                <div className="flex items-center justify-between text-xs">
                  <span className="font-semibold text-sp-text">1. 空间相交与波段能力检查</span>
                  {allSelectedChecked && (
                    <CheckCircle2 className="h-3.5 w-3.5 text-sp-success-text shrink-0" />
                  )}
                </div>
                <button
                  type="button"
                  disabled={!onPreflight || preflightBusy || !draft.requestedGeometry || draft.selectedItemIds.length === 0}
                  aria-busy={preflightBusy}
                  onClick={() => void preflight()}
                  className="inline-flex h-8 w-full items-center justify-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:cursor-not-allowed disabled:opacity-50 transition-colors cursor-pointer"
                >
                  <RefreshCw className={cn("h-3.5 w-3.5 text-sp-text-secondary shrink-0", preflightSpinClass)} />
                  <span>检查相交与能力</span>
                </button>

                {allSelectedChecked && (
                  <div
                    className="grid grid-cols-2 gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted/50 px-3 py-1.5 text-xs tabular-nums"
                    aria-label="批量裁剪检查结果"
                  >
                    <span className="font-semibold text-sp-success-text">
                      可执行 {readiness.executableItemIds.length}
                    </span>
                    <span className="font-medium text-sp-text-muted text-right">
                      排除 {readiness.excludedItemIds.length}
                    </span>
                  </div>
                )}
              </div>

              {/* 步骤 2：频率通道（单选） */}
              <fieldset className="space-y-2 px-3.5 py-3">
                <legend className="text-xs font-semibold text-sp-text">频率通道（单选）</legend>
                {commonSelections.length === 0 ? (
                  <p className="text-xs text-sp-warning-text">
                    {readiness.code === "preflight_required"
                      ? "完成相交与能力检查后显示可选频率。"
                      : "当前所选影像没有共同频率。"}
                  </p>
                ) : (
                  <div className="grid grid-cols-1 gap-1.5">
                    {commonSelections.map((choice) => {
                      const active = draft.selectedFrequency === choice.frequency;
                      return (
                        <label
                          key={choice.frequency}
                          className={cn(
                            "flex items-center justify-between gap-2 rounded-sp-sm border px-3 py-2 text-xs cursor-pointer transition-colors",
                            active
                              ? "border-sp-text/45 bg-sp-selected font-semibold text-sp-text"
                              : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover"
                          )}
                        >
                          <span className="flex items-center gap-2">
                            <input
                              type="radio"
                              name="nisar-subset-frequency"
                              aria-label={`Frequency ${choice.frequency}`}
                              checked={active}
                              disabled={Boolean(taskContext)}
                              onChange={() => selectFrequency(choice.frequency)}
                              className="h-3.5 w-3.5 accent-neutral-900 dark:accent-neutral-100"
                            />
                            <span>Frequency {choice.frequency}</span>
                          </span>
                          <span className="font-mono text-[11px] text-sp-text-muted">
                            {choice.polarizations.join(" / ")}
                          </span>
                        </label>
                      );
                    })}
                  </div>
                )}
              </fieldset>

              {/* 步骤 3：极化方式（可多选） */}
              <fieldset className="space-y-2 px-3.5 py-3">
                <legend className="text-xs font-semibold text-sp-text">极化（可多选）</legend>
                {!selectedChoice ? (
                  <p className="text-xs text-sp-text-muted">先选择频率通道。</p>
                ) : (
                  <div className="grid grid-cols-2 gap-1.5">
                    {selectedChoice.polarizations.map((polarization) => {
                      const checked = draft.selectedPolarizations.includes(polarization);
                      return (
                        <label
                          key={polarization}
                          className={cn(
                            "flex items-center gap-2 rounded-sp-sm border px-2.5 py-1.5 text-xs font-mono cursor-pointer transition-colors",
                            checked
                              ? "border-sp-text/45 bg-sp-selected font-semibold text-sp-text"
                              : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover"
                          )}
                        >
                          <input
                            type="checkbox"
                            aria-label={`Polarization ${polarization}`}
                            checked={checked}
                            disabled={Boolean(taskContext)}
                            onChange={() => togglePolarization(polarization)}
                            className="h-3.5 w-3.5 rounded-xs border-sp-line accent-neutral-900 dark:accent-neutral-100"
                          />
                          <span>{polarization}</span>
                        </label>
                      );
                    })}
                  </div>
                )}
              </fieldset>

              {/* 派生交付说明与科学提示 */}
              <div className="space-y-1.5 px-3.5 py-3 text-xs leading-relaxed text-sp-text-secondary">
                <p>每景实际范围会在后端读取产品结构后按真实足迹计算，不用外接矩形代替。</p>
                <p data-testid="nisar-workspace-estimate-summary">{liveEstimateSummary}</p>
                <p>输出：每景一个客户端派生、PROVISIONAL HDF5。</p>
              </div>

              {((!readiness.overlapVerified && readiness.executableItemIds.length > 0) || readiness.reason || error || reviewError) && (
                <div className="space-y-2 px-3.5 py-3">
                  {!readiness.overlapVerified && readiness.executableItemIds.length > 0 && (
                    <p className="rounded-sp-sm border border-sp-warning-text/20 bg-sp-warning-bg px-2.5 py-1.5 text-xs text-sp-warning-text">
                      相交状态将在“检查并下载”后由正式规划确认。
                    </p>
                  )}
                  {readiness.reason && (
                    <p className="break-words rounded-sp-sm border border-sp-warning-text/20 bg-sp-warning-bg px-2.5 py-1.5 text-xs text-sp-warning-text">
                      {readiness.reason}
                      {conflictingLabels.length > 0 ? `：${conflictingLabels.join("、")}` : ""}
                    </p>
                  )}
                  {error && (
                    <p role="alert" className="break-words rounded-sp-sm border border-sp-danger-text/25 bg-sp-danger-bg px-2.5 py-1.5 text-xs text-sp-danger-text">
                      {error}
                    </p>
                  )}
                  {reviewError && (
                    <p role="alert" className="break-words rounded-sp-sm border border-sp-danger-text/25 bg-sp-danger-bg px-2.5 py-1.5 text-xs text-sp-danger-text">
                      {reviewError}
                    </p>
                  )}
                </div>
              )}
            </div>

            <div className="grid shrink-0 grid-cols-[auto_1fr] gap-2 border-t border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3.5 py-2.5">
              <button
                type="button"
                disabled={reviewBusy}
                onClick={onClose}
                className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-3.5 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:cursor-not-allowed disabled:opacity-50 transition-colors cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                disabled={!readiness.ready || boundaryBusy || reviewBusy || Boolean(taskContext && !taskContext.outputRoot.trim())}
                onClick={() => void onReview(structuredClone(draft), structuredClone(frozenRows))}
                className="eo-settings-btn-primary h-8 rounded-sp-sm px-4 text-xs font-semibold disabled:cursor-not-allowed disabled:opacity-50 transition-opacity cursor-pointer"
              >
                {reviewBusy
                  ? "提交中…"
                  : taskContext
                    ? taskContext.successor
                      ? "确认创建新裁剪任务"
                      : "确认追加"
                    : "检查并下载"}
              </button>
            </div>
          </aside>
        </div>
      </section>

      {boundaryPreview && (
        <div className="fixed inset-0 z-[1300] flex items-center justify-center bg-black/25 p-4">
          <section
            role="dialog"
            aria-modal="true"
            aria-label="选择裁剪边界要素"
            className="flex max-h-[80vh] w-full max-w-lg flex-col overflow-hidden rounded-sp-lg border border-sp-line bg-sp-surface text-sp-text shadow-modal"
          >
            <header className="flex items-center justify-between gap-3 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-4 py-3">
              <div className="min-w-0">
                <h3 className="text-[13px] font-semibold text-sp-text">选择裁剪边界要素</h3>
                <p className="truncate font-mono text-[11px] text-sp-text-muted">
                  {boundaryPreview.file_name} · {boundaryPreview.total_features} 个要素
                </p>
              </div>
              <button
                type="button"
                aria-label="关闭边界要素选择"
                onClick={() => setBoundaryPreview(null)}
                className="flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-muted hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              >
                <X className="h-4 w-4" />
              </button>
            </header>
            <div className="min-h-0 flex-1 divide-y divide-sp-line overflow-y-auto">
              {boundaryPreview.features.length === 0 ? (
                <p className="p-4 text-xs text-sp-warning-text">预览中没有可选择的边界要素。</p>
              ) : (
                boundaryPreview.features.map((feature) => (
                  <label
                    key={feature.id}
                    className="flex items-start gap-2.5 bg-sp-surface px-4 py-2.5 text-xs hover:bg-sp-hover cursor-pointer"
                  >
                    <input
                      type="checkbox"
                      aria-label={feature.name + " 边界要素"}
                      checked={selectedBoundaryFeatureIds.has(feature.id)}
                      onChange={() => toggleBoundaryFeature(feature.id)}
                      className="mt-0.5 h-3.5 w-3.5 rounded-xs border-sp-line accent-neutral-900 dark:accent-neutral-100"
                    />
                    <span className="min-w-0 flex-1 break-words font-medium text-sp-text">
                      {feature.name || "要素 " + String(feature.index)}
                    </span>
                  </label>
                ))
              )}
              {error && (
                <p role="alert" className="break-words px-4 py-2.5 text-xs text-sp-danger-text">
                  {error}
                </p>
              )}
            </div>
            <footer className="grid grid-cols-[auto_1fr] gap-2 border-t border-sp-line bg-[#fafafa] dark:bg-[#141416] px-4 py-2.5">
              <button
                type="button"
                onClick={() => setBoundaryPreview(null)}
                className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-3.5 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
              >
                取消
              </button>
              <button
                type="button"
                onClick={applyBoundaryFeatures}
                className="eo-settings-btn-primary h-8 rounded-sp-sm px-4 text-xs font-semibold cursor-pointer"
              >
                应用所选边界
              </button>
            </footer>
          </section>
        </div>
      )}
    </div>
  );
}
