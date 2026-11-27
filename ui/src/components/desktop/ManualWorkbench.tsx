import React, { useState, useMemo, useCallback, useEffect } from "react";
import {
  Download,
  AlertTriangle,
  Loader2,
  ArrowDown,
  ArrowUp,
  Check,
  Minus,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  Info,
  X,
  Map as MapIcon,
  Trash2,
  PanelRight,
  PanelLeftOpen,
  Eye,
  EyeOff,
  Scissors,
  Layers,
  Clock,
  RefreshCw,
} from "lucide-react";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon, EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import { isNisarRslcProduct } from "@/lib/nisarSubset";
import { useWorkbenchMapPreview } from "../../hooks/useWorkbenchMapPreview";
import { buildIdentityIndex } from "@/shell/api/mapLinkLogic";
import { groupScenesByStack, sceneStackGroupKey } from "@/shell/api/resultGrouping";
import { GacosBatchPreviewPanel } from "./GacosBatchPreviewPanel";
import type { GacosBatchSubmitState } from "@/lib/companionGacosFlow";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { SceneRow, Bbox } from "@/lib/bridge";
import type {
  SourceDescriptor,
  SourceView,
  SearchProgress,
  RowTaskState,
  RowAction,
  TypedError,
  ApplicationTask,
} from "@/shell/api/types";
import { preferCodeExplanation } from "@/shell/api/errorExplanations";
import { possiblyIncompleteLabel } from "@/lib/catalogSearchCountSummary";
import {
  deliveryCounts,
  isLoading,
  isStoppedEarly,
  pageArrival,
  selectAllMatchingAvailable,
  RESULT_PAGE_SIZE,
  type BatchDelivery,
} from "@/shell/api/batchDeliveryLogic";
import {
  sortScenes,
  nextSort,
  DEFAULT_SORT_STATE,
  type SortState,
  type TaskOutcomeNotice,
  selectableIds,
  headerCheckState,
  checkedSummary,
  sortKeysFor,
  allMatchSummary,
  checkStateInScope,
  isRowCheckedInScope,
} from "@/shell/api/resultListLogic";
import { extractCloudCover } from "@/shell/api/rowProjection";
import {
  mapPanelMode,
  showMapButton,
  nextOverlayStateOnEscape,
  shouldMountDockedMap,
  type MapPanelMode,
} from "@/shell/api/mapPanelLogic";
import { isSourceSearchOnly } from "@/shell/api/sourceConditions";
import type { SearchFilterDraft } from "@/lib/searchFilterSchema";
import type { UnifiedSearchRequest } from "@/lib/generated/v3-contracts";
import { buildManualConditionTags, manualConditionsChanged } from "./manualSearchConditions";
import { staleSearchBatchAgeLabel, type ManualConditionsGroup } from "@/shell/api/manualConditionsLogic";
import { WORKBENCH_LOCALE_TEXT } from "@/lib/manualWorkbenchLocale";
import { CatalogResultRow } from "./CatalogResultRow";
import { ManualConditionsPanel } from "./ManualConditionsPanel";
import type { GeeSyncedResultsPayload } from "./GeeCatalogAutocomplete";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import { CompanionDownloadPopover } from "./CompanionDownloadPopover";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { CatalogSkeletonRows } from "@/components/ui/Skeleton";
import {
  type CompanionState,
  getDefaultPreciseOrbit,
  getSavedCompanionOrbitPreference,
  getSavedCompanionDemPreference,
  getSavedCompanionGacosPreference,
  getSavedCompanionExtentMode,
  isSentinel1Resource,
  isSarCompanionResource,
  calculateOrbitMatch,
  resolveAuxiliaryExtent,
  estimateGacosRequests,
} from "@/lib/companionDownloadLogic";
import {
  WorkbenchMap,
  type WorkbenchDrawMode,
  type MapLayerKey,
  type GeodataRasterOverlaySpec,
} from "@/components/WorkbenchMap";
import { ORBIT_LEGEND_LABELS_EN } from "@/shell/api/orbitTerms";
import { isSettingsModalOpen } from "@/shell/api/settingsModalLayer";
import { InplaceLoginCard } from "@/shell/components/InplaceLoginCard";
import {
  collapseManualColumn,
  expandManualColumn,
  readManualColumnState,
  writeManualColumnState,
  resolveManualColumnLayout,
  resizeManualColumns,
  resetManualSplitter,
  MANUAL_COLUMN_LIMITS,
  NARROW_WORKBENCH_THRESHOLD,
  type ManualColumn,
  type ManualColumnSplitter,
} from "./manualColumnState";

export const DEFAULT_SORT_KEYS = [
  { key: "date", labelKey: "mapWorkbench.results.date" },
  { key: "relativeOrbit", label: "Path" },
  { key: "frame", label: "Frame" },
  { key: "size", labelKey: "mapWorkbench.results.size" },
] as const;

export interface ManualWorkbenchProps {
  layoutWidth?: number;
  onContainerRef?: (node: HTMLDivElement | null) => void;
  mapFootprintControls?: (selectScene: (sceneId: string) => void) => React.ReactNode;
  // 条件栏相关
  scientificMode: any;
  onScientificModeChange: (mode: any) => void;
  onResetParams: () => void;
  availableSources: SourceDescriptor[];
  activeResourceId: string;
  onSelectSource: (resourceId: string) => void;
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
  onRefreshSearch?: () => void;
  activeSourceView?: SourceView | null;
  frozenSearchRequest?: UnifiedSearchRequest | null;
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

  // 结果列相关
  searchProgress?: SearchProgress | null;
  scenes: SceneRow[];
  /** Metadata for every checked product, including rows outside the loaded prefix. */
  checkedScenes?: SceneRow[];
  sourceViewLoading?: boolean;
  sourceViewError?: string | null;
  onRetrySourceView?: () => void;
  resultTotalCount?: number;
  loadingMore?: boolean;
  loadMoreError?: string | null;
  onLoadMore?: () => void;
  /** #22 B slice 5: delivery state of the shown batch (loading, complete, stopped early). */
  batchDelivery?: BatchDelivery | null;
  /** A page read answered "not yet loaded"; it is read again when more rows arrive. */
  pageWaiting?: boolean;
  onCancelSearch?: () => void;
  cancellingSearch?: boolean;
  rowStates?: Map<string, RowTaskState>;
  tasksById?: Map<string, ApplicationTask>;
  taskOutcomeNotice?: TaskOutcomeNotice | null;
  onDismissTaskOutcomeNotice?: () => void;
  selectedSceneId: string | null;
  onSelectScene: (sceneId: string | null) => void;
  checkedSceneIds: Set<string>;
  onClearChecked?: () => void;
  onClearResults?: () => void;
  onToggleCheckScene: (sceneId: string) => void;
  onToggleCheckAll: () => void;
  allMatchingSelected?: boolean;
  excludedSceneIds?: Set<string>;
  onToggleSelectCurrentPage?: (pageSceneIds: string[]) => void;
  onToggleSelectAllMatching?: () => void;
  onRowAction?: (itemId: string, action: RowAction) => void;
  onDownloadChecked: () => void;
  downloadBusy: boolean;
  downloadDisabledReason?: string | null;
  creatingItemIds?: Set<string>;
  batchCreating?: boolean;
  onOpenNisarSubsetWorkspace?: () => void;
  nisarSubsetBusy?: boolean;
  nisarSubsetDisabledReason?: string | null;
  canBatchOpticalAoiClip?: boolean;
  opticalAoiClipDisabledReason?: string | null;
  onOpenBatchOpticalAoiClip?: () => void;
  canRowOpticalAoiClip?: (scene: SceneRow) => boolean;
  onOpenRowOpticalAoiClip?: (sceneId: string) => void;

  // 地图列相关
  drawMode: WorkbenchDrawMode;
  drawActive: boolean;
  onDrawModeChange: (mode: WorkbenchDrawMode) => void;
  onDrawActiveChange: (active: boolean) => void;
  onClearLayers: () => void;
  onRectDraw: (bbox: Bbox) => void;
  onPolygonDraw: (ring: [number, number][]) => void;
  onPointDraw: (lat: number, lng: number) => void;
  mapLayerKey: MapLayerKey;
  onLayerChange: (layer: MapLayerKey) => void;
  onOpenTileDownloadWorkbench?: (suggestedTileSource: string) => void;
  geodataRasterOverlay?: GeodataRasterOverlaySpec | null;
  onPreviewWaybackOverlay?: (overlay: GeodataRasterOverlaySpec | null) => void;

  // 辅助组件浮层
  inlineLoginError?: TypedError | null;
  onCloseInlineLogin?: () => void;
  onSuccessInlineLogin?: () => Promise<void>;
  onOpenSettings?: () => void;
  onOpenCatalog?: (opts?: { geeOnly?: boolean }) => void;
  companionState?: CompanionState;
  onCompanionStateChange?: (next: CompanionState) => void;
  onTriggerStandaloneCompanion?: () => void | Promise<void>;
  standaloneCompanionBusy?: boolean;
  hideMultiProviderSelector?: boolean;
  onConditionRemoved?: (removedId: string) => void;
  onSyncGeeResults?: (payload: GeeSyncedResultsPayload) => void;
  onPreviewGeeScene?: (scene: SceneRow) => void;
  gacosBatchStatuses?: ReadonlyMap<number, GacosBatchSubmitState>;
  selectedGacosBatchIndices?: readonly number[] | null;
  onToggleGacosBatchIndex?: (batchIndex: number) => void;
  onSelectAllGacosBatches?: (totalBatches: number) => void;
  onRetryGacosBatch?: (batchIndex: number, dates: readonly string[]) => void;
  gacosSubmitting?: boolean;
  forceExpandSearchSignal?: number;
}

export function ManualWorkbench({
  layoutWidth,
  onContainerRef,
  mapFootprintControls,
  scientificMode,
  onScientificModeChange,
  onResetParams,
  availableSources,
  activeResourceId,
  onSelectSource,
  platforms,
  onPlatformsChange,
  productType,
  onProductTypeChange,
  polarizations,
  onPolarizationsChange,
  beamModes,
  onBeamModesChange,
  lookDirection,
  onLookDirectionChange,
  opticalDraft,
  onOpticalDraftChange,
  workingAoi,
  workingGeometry,
  aoiLabel,
  onReplaceAoi,
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
  onRefreshSearch,
  activeSourceView,
  frozenSearchRequest = null,
  demSource,
  onDemSourceChange,
  autoConvertEllipsoid,
  onToggleAutoConvertEllipsoid,
  orbitSourceType,
  onOrbitSourceTypeChange,
  localSlcPath,
  onChangeLocalSlcPath,
  onPickLocalSlcPath,
  localDemPath,
  onChangeLocalDemPath,
  onPickLocalDemPath,
  inlineToolResult,
  onDismissToolResult,
  searchProgress,
  scenes,
  checkedScenes,
  sourceViewLoading = false,
  sourceViewError = null,
  onRetrySourceView,
  resultTotalCount,
  loadingMore = false,
  loadMoreError,
  onLoadMore,
  batchDelivery = null,
  pageWaiting = false,
  onCancelSearch,
  cancellingSearch = false,
  rowStates,
  tasksById,
  taskOutcomeNotice,
  onDismissTaskOutcomeNotice,
  selectedSceneId,
  onSelectScene,
  checkedSceneIds,
  onClearChecked,
  onClearResults,
  onToggleCheckScene,
  onToggleCheckAll,
  allMatchingSelected = false,
  excludedSceneIds,
  onToggleSelectCurrentPage,
  onToggleSelectAllMatching,
  onRowAction,
  onDownloadChecked,
  downloadBusy,
  downloadDisabledReason,
  creatingItemIds,
  batchCreating,
  onOpenNisarSubsetWorkspace,
  nisarSubsetBusy = false,
  nisarSubsetDisabledReason = null,
  canBatchOpticalAoiClip = false,
  opticalAoiClipDisabledReason = null,
  onOpenBatchOpticalAoiClip,
  canRowOpticalAoiClip,
  onOpenRowOpticalAoiClip,
  drawMode,
  drawActive,
  onDrawModeChange,
  onDrawActiveChange,
  onClearLayers,
  onRectDraw,
  onPolygonDraw,
  onPointDraw,
  mapLayerKey,
  onLayerChange,
  onOpenTileDownloadWorkbench,
  geodataRasterOverlay = null,
  onPreviewWaybackOverlay,
  inlineLoginError,
  onCloseInlineLogin,
  onSuccessInlineLogin,
  onOpenSettings,
  onOpenCatalog,
  companionState: externalCompanionState,
  onCompanionStateChange,
  onTriggerStandaloneCompanion,
  standaloneCompanionBusy = false,
  hideMultiProviderSelector = false,
  onConditionRemoved,
  onSyncGeeResults,
  onPreviewGeeScene,
  gacosBatchStatuses,
  selectedGacosBatchIndices,
  onToggleGacosBatchIndex,
  onSelectAllGacosBatches,
  onRetryGacosBatch,
  gacosSubmitting = false,
  forceExpandSearchSignal = 0,
}: ManualWorkbenchProps) {
  const { t, locale } = useI18n();
  const L = WORKBENCH_LOCALE_TEXT[locale];
  // R3(d): 仅在真正的批次检索进行中显示搜索状态，工具模式不触发
  const isSearching = Boolean(searchProgress && !searchProgress.terminal);
  // #22 B slice 3(a): the done search froze a possibly incomplete batch.
  // #22 B slice 5: a batch cancelled, failed or interrupted after its first
  // page keeps the rows that arrived and is possibly incomplete as well.
  const possiblyIncomplete = Boolean(
    searchProgress?.terminal
      && searchProgress.stage === "done"
      && searchProgress.possibly_incomplete === true
      && searchProgress.resource_id === activeSourceView?.resource_id,
  ) || isStoppedEarly(batchDelivery);
  const deliveryLoading = isLoading(batchDelivery);
  const hasPreviousBatch = Boolean(activeSourceView?.batch_id) && !deliveryLoading;
  const hasLastErrorWithBatch = Boolean(activeSourceView?.last_error && activeSourceView?.batch_id);
  const totalCount = resultTotalCount ?? scenes.length;
  const counts = deliveryCounts(batchDelivery, totalCount);
  const hasMoreResults = scenes.length < totalCount;
  const staleBatchAgeLabel = useMemo(
    () => staleSearchBatchAgeLabel(activeSourceView?.searched_at),
    [activeSourceView?.searched_at, locale]
  );
  const showStaleBatchBanner =
    !isSearching &&
    !deliveryLoading &&
    !sourceViewLoading &&
    !sourceViewError &&
    !hasLastErrorWithBatch &&
    scientificMode === "sar" &&
    activeResourceId !== "atmospheric.gacos.ztd.v1" &&
    activeResourceId !== "orbit.sentinel-1.poeorb.v1" &&
    Boolean(activeSourceView?.batch_id) &&
    counts.headline > 0 &&
    staleBatchAgeLabel !== null;
  const canRefreshStaleBatch =
    Boolean(workingAoi) ||
    activeResourceId === "orbit.nisar.oe.v1" ||
    activeResourceId === "orbit.copernicus.pod.v1";

  // 排序与分页状态
  const [sortState, setSortState] = useState<SortState>(DEFAULT_SORT_STATE);
  const [columns, setColumns] = useState(readManualColumnState);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window !== "undefined" ? window.innerWidth : 1920
  );
  const mapButtonRef = React.useRef<HTMLButtonElement>(null);
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(layoutWidth ?? 0);
  const [draggingSplitter, setDraggingSplitter] = useState<ManualColumnSplitter | null>(null);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const handleResize = () => setViewportWidth(window.innerWidth);
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  React.useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const measure = () => setContainerWidth(container.clientWidth || layoutWidth || 0);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [layoutWidth]);

  const columnLayout = resolveManualColumnLayout(columns, containerWidth);
  const expandedColumnCount = Object.values(columnLayout.collapsed).filter((collapsed) => !collapsed).length;
  const panelMode: MapPanelMode = mapPanelMode(viewportWidth, columnLayout.collapsed.map, overlayOpen);

  const autoCollapsedResultsForXyzRef = React.useRef(false);
  const autoCollapsedNarrowSearchRef = React.useRef(false);
  const autoCollapsedNarrowResultsRef = React.useRef(false);

  const collapseColumn = useCallback((column: ManualColumn) => {
    if (column === "results") {
      autoCollapsedResultsForXyzRef.current = false;
      autoCollapsedNarrowResultsRef.current = false;
    } else if (column === "search") {
      autoCollapsedNarrowSearchRef.current = false;
    }
    setColumns((current) => {
      const next = collapseManualColumn(current, column);
      writeManualColumnState(next);
      return next;
    });
  }, []);

  const expandColumn = useCallback((column: ManualColumn) => {
    if (column === "results") {
      autoCollapsedResultsForXyzRef.current = false;
      autoCollapsedNarrowResultsRef.current = false;
    } else if (column === "search") {
      autoCollapsedNarrowSearchRef.current = false;
    }
    setColumns((current) => {
      let next = expandManualColumn(current, column);
      const isNarrow = containerWidth > 0 && containerWidth < NARROW_WORKBENCH_THRESHOLD;
      if (isNarrow) {
        if (column === "search" && !next.collapsed.results) {
          autoCollapsedNarrowResultsRef.current = true;
          next = collapseManualColumn(next, "results");
        } else if (column === "results" && !next.collapsed.search) {
          autoCollapsedNarrowSearchRef.current = true;
          next = collapseManualColumn(next, "search");
        }
        return next;
      }
      writeManualColumnState(next);
      return next;
    });
  }, [containerWidth]);

  React.useEffect(() => {
    if (forceExpandSearchSignal > 0) {
      expandColumn("search");
    }
  }, [forceExpandSearchSignal, expandColumn]);

  React.useEffect(() => {
    if (containerWidth <= 0) return;
    if (containerWidth < NARROW_WORKBENCH_THRESHOLD) {
      setColumns((current) => {
        if (current.collapsed.search || current.collapsed.results) {
          return current;
        }
        if (scenes.length > 0 || isSearching) {
          autoCollapsedNarrowSearchRef.current = true;
          return collapseManualColumn(current, "search");
        }
        autoCollapsedNarrowResultsRef.current = true;
        return collapseManualColumn(current, "results");
      });
      return;
    }

    const restoreSearch = autoCollapsedNarrowSearchRef.current;
    const restoreResults =
      autoCollapsedNarrowResultsRef.current &&
      !autoCollapsedResultsForXyzRef.current &&
      activeResourceId !== "auxiliary.xyz-tiles.v1";
    autoCollapsedNarrowSearchRef.current = false;
    autoCollapsedNarrowResultsRef.current = false;
    if (restoreSearch || restoreResults) {
      setColumns((current) => {
        let next = current;
        if (restoreSearch) next = expandManualColumn(next, "search");
        if (restoreResults) next = expandManualColumn(next, "results");
        return next;
      });
    }
  }, [containerWidth, scenes.length, isSearching, activeResourceId]);

  // 切换至互联网地图瓦片下载（无需右侧逐景检索清单）时自动临时收纳右侧结果栏，切离其他影像源时自动恢复
  React.useEffect(() => {
    if (activeResourceId === "auxiliary.xyz-tiles.v1") {
      setColumns((current) => {
        if (current.collapsed.results) return current;
        autoCollapsedResultsForXyzRef.current = true;
        return collapseManualColumn(current, "results");
      });
    } else if (autoCollapsedResultsForXyzRef.current) {
      autoCollapsedResultsForXyzRef.current = false;
      setColumns((current) => expandManualColumn(current, "results"));
    }
  }, [activeResourceId]);

  const handleWrappedTriggerSearch = useCallback(() => {
    const isNarrow = containerWidth > 0 && containerWidth < NARROW_WORKBENCH_THRESHOLD;
    if (columns.collapsed.results || (isNarrow && !columns.collapsed.search)) {
      expandColumn("results");
    }
    onTriggerSearch();
  }, [columns.collapsed.results, columns.collapsed.search, containerWidth, expandColumn, onTriggerSearch]);

  const handleSyncedGeeResults = useCallback((payload: GeeSyncedResultsPayload) => {
    expandColumn("results");
    onSyncGeeResults?.(payload);
  }, [expandColumn, onSyncGeeResults]);

  const prevSearchingRef = React.useRef(searching);
  React.useEffect(() => {
    if (!prevSearchingRef.current && searching) {
      const isNarrow = containerWidth > 0 && containerWidth < NARROW_WORKBENCH_THRESHOLD;
      if (columns.collapsed.results || (isNarrow && !columns.collapsed.search)) {
        expandColumn("results");
      }
    }
    prevSearchingRef.current = searching;
  }, [searching, columns.collapsed.results, columns.collapsed.search, containerWidth, expandColumn]);

  React.useEffect(() => {
    if (panelMode !== "overlay") return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        // 偏好设置打开时让位，否则捕获阶段的 stopPropagation 会吞掉设置弹窗的 Esc
        if (event.defaultPrevented || isSettingsModalOpen()) return;
        event.stopPropagation();
        setOverlayOpen((prev) => nextOverlayStateOnEscape(prev));
        mapButtonRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [panelMode]);

  const startSplitterDrag = useCallback((event: React.PointerEvent<HTMLDivElement>, splitter: ManualColumnSplitter) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const handle = event.currentTarget;
    const prevBodyCursor = typeof document !== "undefined" ? document.body.style.cursor : "";
    const prevBodyUserSelect = typeof document !== "undefined" ? document.body.style.userSelect : "";
    if (typeof document !== "undefined") {
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    }
    try {
      handle.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is an enhancement; window listeners still track the drag.
    }
    setDraggingSplitter(splitter);
    const startX = event.clientX;
    const initialState = columns;
    let latestState = initialState;
    let rafId: number | null = null;
    const onPointerMove = (nextEvent: PointerEvent) => {
      const deltaX = nextEvent.clientX - startX;
      if (rafId !== null) cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(() => {
        const next = resizeManualColumns(initialState, splitter, deltaX, containerWidth);
        latestState = next;
        setColumns(next);
      });
    };
    const onPointerUp = (nextEvent: PointerEvent) => {
      if (rafId !== null) cancelAnimationFrame(rafId);
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      if (typeof document !== "undefined") {
        document.body.style.cursor = prevBodyCursor;
        document.body.style.userSelect = prevBodyUserSelect;
      }
      try {
        handle.releasePointerCapture(nextEvent.pointerId);
      } catch {
        // The pointer may already have been released.
      }
      setDraggingSplitter(null);
      writeManualColumnState(latestState);
    };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  }, [columns, containerWidth]);

  const resizeSplitterWithKeyboard = useCallback((event: React.KeyboardEvent<HTMLDivElement>, splitter: ManualColumnSplitter) => {
    const direction = event.key === "ArrowLeft" ? -16 : event.key === "ArrowRight" ? 16 : 0;
    if (direction === 0 && event.key !== "Enter" && event.key !== "Home") return;
    event.preventDefault();
    setColumns((current) => {
      const next = direction === 0
        ? resetManualSplitter(current, splitter)
        : resizeManualColumns(current, splitter, direction, containerWidth);
      writeManualColumnState(next);
      return next;
    });
  }, [containerWidth]);

  const renderSplitter = (splitter: ManualColumnSplitter) => {
    const isSearch = splitter === "search-map";
    const column = isSearch ? "search" : "results";
    const value = columnLayout.collapsed[column] ? 36 : columnLayout.widths[column];
    return (
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={isSearch ? t("mapWorkbench.map.resizeSearchMap") : t("mapWorkbench.map.resizeMapResults")}
        aria-valuenow={value}
        aria-valuemin={36}
        aria-valuemax={isSearch ? MANUAL_COLUMN_LIMITS.search.max : undefined}
        aria-valuetext={columnLayout.collapsed[column] ? t("mapWorkbench.map.collapsedValue") : t("mapWorkbench.map.pixelsValue", { value })}
        tabIndex={0}
        onPointerDown={(event) => startSplitterDrag(event, splitter)}
        onKeyDown={(event) => resizeSplitterWithKeyboard(event, splitter)}
        onDoubleClick={() => setColumns((current) => {
          const next = resetManualSplitter(current, splitter);
          writeManualColumnState(next);
          return next;
        })}
        className="group relative z-40 flex h-full w-2 shrink-0 cursor-col-resize items-center justify-center select-none outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
      >
        <div className={cn(
          "h-full w-px transition-colors duration-fast ease-sp",
          draggingSplitter === splitter ? "bg-sp-accent w-0.5" : "bg-transparent group-hover:bg-sp-line group-focus-visible:bg-sp-line",
        )} />
      </div>
    );
  };
  const [currentPage, setCurrentPage] = useState<number>(1);

  // 失败原因展开状态管理（以 scene_id 为键）
  const [expandedFailureSceneIds, setExpandedFailureSceneIds] = useState<Set<string>>(new Set());
  const handleToggleFailureExpand = useCallback((sceneId: string) => {
    setExpandedFailureSceneIds((prev) => {
      const next = new Set(prev);
      if (next.has(sceneId)) next.delete(sceneId);
      else next.add(sceneId);
      return next;
    });
  }, []);

  const activeSourceDesc = useMemo(
    () => availableSources.find((s) => s.resource_id === activeResourceId) || null,
    [availableSources, activeResourceId]
  );
  const activeSourceFamily = activeSourceDesc?.family || "sar";
  const conditionDraft = {
    resourceId: activeResourceId,
    aoiGeometry: workingGeometry,
    aoiName: aoiLabel,
    startDate,
    endDate,
    platforms,
    productType,
    polarizations,
    beamModes,
    lookDirection,
    orbitDirection: orbitDirection || "all",
    relativeOrbit,
    path,
    frame,
    opticalDraft,
  };
  const conditionTags = useMemo(() => frozenSearchRequest
    ? buildManualConditionTags(frozenSearchRequest)
    : [], [frozenSearchRequest, locale]);
  const conditionsChanged = manualConditionsChanged(frozenSearchRequest, conditionDraft);
  const [catalogOpen, setCatalogOpen] = useState(false);
  const [catalogGeeOnly, setCatalogGeeOnly] = useState(false);
  const [companionPopoverOpen, setCompanionPopoverOpen] = useState(false);
  const [internalCompanionState, setInternalCompanionState] = useState<CompanionState>(() => ({
    preciseOrbit: getSavedCompanionOrbitPreference(activeResourceId) ?? getDefaultPreciseOrbit(activeResourceId),
    dem: getSavedCompanionDemPreference(activeResourceId),
    gacos: getSavedCompanionGacosPreference(activeResourceId),
    extentMode: getSavedCompanionExtentMode(activeResourceId),
  }));
  const companionState = externalCompanionState ?? internalCompanionState;
  const setCompanionState = useCallback(
    (next: CompanionState | ((prev: CompanionState) => CompanionState)) => {
      const resolved = typeof next === "function" ? next(companionState) : next;
      if (onCompanionStateChange) {
        onCompanionStateChange(resolved);
      } else {
        setInternalCompanionState(resolved);
      }
    },
    [companionState, onCompanionStateChange]
  );

  useEffect(() => {
    const savedOrbit = getSavedCompanionOrbitPreference(activeResourceId);
    const savedDem = getSavedCompanionDemPreference(activeResourceId);
    const savedGacos = getSavedCompanionGacosPreference(activeResourceId);
    const savedExtent = getSavedCompanionExtentMode(activeResourceId);
    setCompanionState((prev) => ({
      ...prev,
      preciseOrbit: savedOrbit ?? getDefaultPreciseOrbit(activeResourceId),
      dem: savedDem,
      gacos: savedGacos,
      extentMode: savedExtent,
    }));
  }, [activeResourceId]);

  const checkedScenesList = useMemo(() => {
    return scenes.filter((s) => checkedSceneIds.has(s.scene_id));
  }, [scenes, checkedSceneIds]);

  const orbitMatch = useMemo(() => {
    return calculateOrbitMatch(checkedScenesList, activeResourceId);
  }, [checkedScenesList, activeResourceId]);

  const gacosEstimate = useMemo(() => {
    const eff = resolveAuxiliaryExtent(companionState.extentMode, workingAoi, checkedScenesList);
    return estimateGacosRequests(checkedScenesList, eff);
  }, [checkedScenesList, companionState.extentMode, workingAoi]);

  const isSentinel1 = isSentinel1Resource(activeResourceId);
  const isSarCompanion = isSarCompanionResource(activeResourceId, activeSourceFamily);
  const isNisarSource = useMemo(
    () =>
      scientificMode !== "gee"
      && (
        activeResourceId === "science.nisar.l.v1"
        || activeResourceId === "science.nisar.s.v1"
        || activeResourceId === "science.nisar.l.rslc.provisional.v1"
        || activeResourceId === "science.nisar.s.ssar.rslc.v1"
        || activeResourceId === "sar.nisar.l.rslc.provisional.v1"
        || activeResourceId === "sar.nisar.s.rslc.provisional.v1"
        || activeResourceId === "sar.nisar.s.ssar.rslc.v1"
        || scenes.some((s) => isNisarRslcProduct(s.sar_product))
      ),
    [scientificMode, activeResourceId, scenes],
  );
  // A newly shown batch starts on its first page, and its live batchId drives map/row quicklooks before sourceViews hydrates.
  const shownBatchId = batchDelivery?.batchId ?? activeSourceView?.batch_id ?? null;

  const companionButtonLabel = useMemo(() => {
    const items: string[] = [];
    if (companionState.preciseOrbit && isSentinel1) items.push(t("panels.companionDownload.preciseOrbitTitle"));
    if (companionState.dem) items.push("DEM");
    if (companionState.gacos) items.push("GACOS");
    if (items.length === 0) return t("panels.companionDownload.companionButtonText");
    if (items.length === 1) return t("panels.companionDownload.companionButtonTextSingle", { name: items[0] });
    return t("panels.companionDownload.companionButtonTextMultiple", { count: items.length });
  }, [companionState, isSentinel1, t]);

  useEffect(() => {
    const handleGlobalCatalogKey = (e: KeyboardEvent) => {
      if (onOpenCatalog) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCatalogGeeOnly(scientificMode === "gee");
        setCatalogOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleGlobalCatalogKey);
    return () => window.removeEventListener("keydown", handleGlobalCatalogKey);
  }, [onOpenCatalog, scientificMode]);

  const [focusedCondition, setFocusedCondition] = useState<{
    group: ManualConditionsGroup;
    field?: string;
    requestId: string;
  } | null>(null);
  const focusCondition = useCallback((group: ManualConditionsGroup, field?: string) => {
    if (frozenSearchRequest) {
      setFocusedCondition({ group, field, requestId: frozenSearchRequest.request_id });
    }
    expandColumn("search");
  }, [expandColumn, frozenSearchRequest]);
  const removeCondition = useCallback((id: string) => {
    const item = conditionTags.find((candidate) => candidate.id === id);
    if (!item?.removable) return;
    switch (id) {
      case "aoi":
        void Promise.resolve(onReplaceAoi(null, "", null)).catch(() => {});
        break;
      case "time":
        onStartDateChange("");
        onEndDateChange("");
        break;
      case "platforms": onPlatformsChange?.([]); break;
      case "productType": onProductTypeChange(""); break;
      case "polarizations": onPolarizationsChange?.([]); break;
      case "beamModes": onBeamModesChange?.([]); break;
      case "lookDirection": onLookDirectionChange?.(null); break;
      case "orbitDirection": onOrbitDirectionChange("all"); break;
      case "relativeOrbit": onRelativeOrbitChange(""); break;
      case "path": onPathChange?.(""); break;
      case "frame": onFrameChange(""); break;
      default:
        if (id.startsWith("optical:")) {
          const filterId = id.slice("optical:".length);
          onOpticalDraftChange?.({ ...(opticalDraft ?? {}), [filterId]: undefined });
        }
    }
    onConditionRemoved?.(id);
  }, [conditionTags, onReplaceAoi, onStartDateChange, onEndDateChange, onPlatformsChange,
    onProductTypeChange, onPolarizationsChange, onBeamModesChange, onLookDirectionChange, onOrbitDirectionChange, onRelativeOrbitChange,
    onPathChange, onFrameChange, opticalDraft, onOpticalDraftChange, onConditionRemoved]);
  const isSearchOnly = useMemo(
    () => isSourceSearchOnly(activeResourceId),
    [activeResourceId]
  );
  const hasCloud = useMemo(
    () => scenes.some((s) => extractCloudCover(s) !== null),
    [scenes]
  );
  const availableSortKeys = useMemo(
    () => sortKeysFor(activeSourceFamily === "optical" ? "optical" : "sar", hasCloud),
    [activeSourceFamily, hasCloud, locale]
  );

  // 客户端排序与分页
  const effectiveSortKey = availableSortKeys.some((k) => k.key === sortState.key)
    ? sortState.key
    : "date";

  const sortedScenes = useMemo(() => {
    return sortScenes(scenes, effectiveSortKey, sortState.dir);
  }, [scenes, effectiveSortKey, sortState.dir]);

  // Pages reach every row the batch can hold, also rows that have not been
  // read (or have not arrived) yet; such a page shows a loading state.
  const paginated = useMemo(() => {
    const pageCount = Math.max(1, Math.ceil(Math.max(sortedScenes.length, counts.pageable) / RESULT_PAGE_SIZE));
    const page = Math.min(Math.max(1, Math.floor(currentPage) || 1), pageCount);
    const start = (page - 1) * RESULT_PAGE_SIZE;
    return { items: sortedScenes.slice(start, start + RESULT_PAGE_SIZE), page, pageCount };
  }, [sortedScenes, currentPage, counts.pageable]);
  const arrival = pageArrival(scenes.length, paginated.page, counts.pageable);
  const showSearchProgressSpinner = useDelayedFlag(isSearching || deliveryLoading);
  const showBatchCreatingSpinner = useDelayedFlag(Boolean(downloadBusy || batchCreating));
  const showNisarSubsetSpinner = useDelayedFlag(nisarSubsetBusy);
  const showSourceViewSkeleton = useDelayedFlag(sourceViewLoading);
  const showResultPageSkeleton = useDelayedFlag(arrival !== "ready" && !loadMoreError);
  const showLoadMoreSpinner = useDelayedFlag(loadingMore);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [shownBatchId]);

  React.useEffect(() => {
    if (sourceViewLoading || sourceViewError || arrival === "ready" || loadingMore || loadMoreError || pageWaiting) return;
    onLoadMore?.();
  }, [arrival, loadingMore, loadMoreError, pageWaiting, onLoadMore, scenes.length, sourceViewLoading, sourceViewError]);

  // 可选行与三态表头复选框计算（本页 / 已加载 / 全部匹配）
  const allMatchState = useMemo(
    () => (allMatchingSelected ? { active: true, excludedIds: excludedSceneIds ?? new Set<string>() } : null),
    [allMatchingSelected, excludedSceneIds],
  );
  const pageSelectable = useMemo(() => {
    return selectableIds(paginated.items, rowStates || new Map());
  }, [paginated.items, rowStates]);
  const pageCheckState = useMemo(() => {
    return checkStateInScope(pageSelectable, checkedSceneIds, allMatchState);
  }, [pageSelectable, checkedSceneIds, allMatchState]);
  const selectable = useMemo(() => {
    return selectableIds(scenes, rowStates || new Map());
  }, [scenes, rowStates]);

  const checkState = useMemo(() => {
    return checkStateInScope(selectable, checkedSceneIds, allMatchState);
  }, [checkedSceneIds, selectable, allMatchState]);

  const summary = useMemo(() => {
    return allMatchingSelected
      ? allMatchSummary(counts.headline, scenes, rowStates || new Map(), excludedSceneIds ?? new Set())
      : checkedSummary(checkedScenes ?? scenes, checkedSceneIds);
  }, [allMatchingSelected, counts.headline, scenes, rowStates, excludedSceneIds, checkedScenes, checkedSceneIds, locale]);

  // W5-3e: 悬停高亮、同轨同幅分堆、快视图与点选定位（统一复用 useWorkbenchMapPreview）
  const [adminPreviewGeometry, setAdminPreviewGeometry] = useState<Record<string, unknown> | null>(null);
  // #22C: 同幅同框时序栈聚合视图开关（支持可折叠与整组全选）
  const [groupByStack, setGroupByStack] = useState<boolean>(false);
  const [collapsedStackKeys, setCollapsedStackKeys] = useState<Set<string>>(new Set());
  const toggleStackCollapse = useCallback((key: string) => {
    setCollapsedStackKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const {
    mapPreviewEnabled,
    setMapPreviewEnabled,
    pinnedPreviewIds,
    pinnedPreviews,
    togglePinPreviewScene,
    clearPinnedPreviews,
    handlePinnedPreviewError,
    hoveredSceneId,
    setHoveredSceneId,
    selectedSceneObj,
    hasMapPreviewAvailable,
    activePreviewTileLayer,
    activePreviewKey,
    selectedScenePreviewBbox,
    mapPreviewState,
    identityIndex,
    stackScenesFor,
    handleSceneSelectFromMap,
  } = useWorkbenchMapPreview({
    scenes,
    selectableScenes: paginated.items,
    selectedSceneId,
    batchId: shownBatchId,
    onSelectScene,
    explicitToggleMode: true,
  });

  const groupedSarStacks = useMemo(() => {
    if (!groupByStack || scientificMode === "gee") return null;
    return groupScenesByStack(paginated.items, { sortByStackSize: true });
  }, [groupByStack, scientificMode, paginated.items]);

  React.useEffect(() => {
    if (!selectedSceneId || !groupByStack) return;
    const match = paginated.items.find((s) => s.scene_id === selectedSceneId);
    if (!match) return;
    const targetKey = sceneStackGroupKey(match);
    setCollapsedStackKeys((prev) => {
      if (!prev.has(targetKey)) return prev;
      const next = new Set(prev);
      next.delete(targetKey);
      return next;
    });
  }, [selectedSceneId, groupByStack, paginated.items]);

  const resultsScrollRef = React.useRef<HTMLDivElement | null>(null);
  const firstSceneId = scenes[0]?.scene_id ?? null;
  const activeBatchResetKey = `${activeResourceId}::${activeSourceView?.batch_id ?? ""}::${firstSceneId ?? ""}`;
  React.useLayoutEffect(() => {
    if (resultsScrollRef.current) {
      resultsScrollRef.current.scrollTop = 0;
    }
  }, [activeBatchResetKey]);

  React.useEffect(() => {
    if (!selectedSceneId) return;
    const timer = setTimeout(() => {
      const escapedSceneId = window.CSS?.escape ? window.CSS.escape(selectedSceneId) : selectedSceneId.replaceAll('"', '\\"');
      const el = document.querySelector(`[data-scene-id="${escapedSceneId}"]`);
      if (el) {
        const container = resultsScrollRef.current;
        if (selectedSceneId === firstSceneId && container && container.scrollTop > 0) {
          container.scrollTop = 0;
        }
        if (container) {
          const cRect = container.getBoundingClientRect();
          const eRect = el.getBoundingClientRect();
          if (cRect.height > 0 && eRect.height > 0 && eRect.top >= cRect.top && eRect.bottom <= cRect.bottom) {
            return;
          }
        }
        const prefersReducedMotion =
          typeof window !== "undefined" &&
          typeof window.matchMedia === "function" &&
          window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        el.scrollIntoView({
          block: "nearest",
          behavior: prefersReducedMotion ? "auto" : "smooth",
        });
      }
    }, 50);
    return () => clearTimeout(timer);
  }, [selectedSceneId, currentPage, firstSceneId]);

  const prevResourceIdRef = React.useRef(activeResourceId);
  React.useEffect(() => {
    if (prevResourceIdRef.current !== activeResourceId) {
      prevResourceIdRef.current = activeResourceId;
      setHoveredSceneId(null);
    }
  }, [activeResourceId, setHoveredSceneId]);

  const renderWorkbenchMap = () => (
    <WorkbenchMap
      declutter
      chrome="panel"
      disableSceneHoverCard
      fitScope="aoi"
      lockViewportOnSceneChange={true}
      popupTitleMode="display_name"
      hideDownloadEntry
      bbox={workingAoi}
      aoiBbox={workingAoi}
      aoiGeometry={workingGeometry}
      aoiName={aoiLabel}
      adminPreviewGeometry={adminPreviewGeometry}
      geodataRasterOverlay={geodataRasterOverlay}
      strictWorkspaceAoi={true}
      orbitLabels={{ ...ORBIT_LEGEND_LABELS_EN, heading: "Orbit direction" }}
      scenes={scenes}
      selectedSceneId={
        selectedSceneId ? (identityIndex.toIdentity.get(selectedSceneId) ?? selectedSceneId) : null
      }
      hoveredSceneId={
        hoveredSceneId ? (identityIndex.toIdentity.get(hoveredSceneId) ?? hoveredSceneId) : null
      }
      preview={mapPreviewState}
      pinnedPreviews={mapPreviewEnabled ? pinnedPreviews : []}
      onPinnedPreviewError={handlePinnedPreviewError}
      previewTileLayer={activePreviewTileLayer}
      activePreviewProductKey={activePreviewKey}
      previewBbox={mapPreviewEnabled && hasMapPreviewAvailable ? selectedScenePreviewBbox : null}
      previewGeometry={
        mapPreviewEnabled && hasMapPreviewAvailable
          ? ((pinnedPreviews[pinnedPreviews.length - 1]?.footprintGeojson ?? selectedSceneObj?.footprint_geojson ?? null) as any)
          : null
      }
      stackScenesFor={stackScenesFor}
      onSceneSelect={(s: unknown) =>
        handleSceneSelectFromMap(typeof s === "string" ? s : (s as { scene_id?: string })?.scene_id || null)
      }
      drawMode={drawMode}
      drawActive={drawActive}
      onDrawModeChange={onDrawModeChange}
      onDrawActiveChange={onDrawActiveChange}
      onClearLayers={onClearLayers}
      onRectDraw={onRectDraw}
      onPolygonDraw={onPolygonDraw}
      onPointDraw={onPointDraw}
      onVectorAoiImported={onImportVectorSuccess}
      layerKey={mapLayerKey}
      onLayerChange={onLayerChange}
      onOpenTileDownloadWorkbench={onOpenTileDownloadWorkbench}
      topControls={
        <>
          {(hasMapPreviewAvailable || pinnedPreviewIds.size > 0) && (
            <div className="pointer-events-auto inline-flex items-center gap-1">
              <button
                type="button"
                data-testid="btn-toggle-map-preview"
                onClick={() => setMapPreviewEnabled((prev) => !prev)}
                title={mapPreviewEnabled ? L.resultsExtra.hidePreviewTitle : L.resultsExtra.showPreviewTitle}
                className={cn(
                  "h-7 px-2.5 rounded-full text-xs font-medium inline-flex items-center gap-1.5 shadow-overlay transition-colors cursor-pointer",
                  mapPreviewEnabled
                    ? "bg-sp-accent-fill text-white"
                    : "bg-sp-surface/90 text-sp-text-secondary hover:text-sp-text"
                )}
              >
                {mapPreviewEnabled ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
                <span>
                  {(mapPreviewEnabled
                    ? L.resultsExtra.mapPreviewBadge
                    : L.resultsExtra.mapPreviewHiddenBadge
                  ).replace("{count}", String(Math.max(pinnedPreviews.length, pinnedPreviewIds.size)))}
                </span>
              </button>
              <button
                type="button"
                data-testid="btn-clear-map-preview"
                onClick={clearPinnedPreviews}
                title={L.resultsExtra.clearPreviewsAria}
                aria-label={L.resultsExtra.clearPreviewsAria}
                className="h-7 w-7 rounded-full bg-sp-surface/90 text-sp-text-secondary hover:text-sp-text inline-flex items-center justify-center shadow-overlay transition-colors cursor-pointer"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}
          {mapFootprintControls?.(handleSceneSelectFromMap)}
        </>
      }
    />
  );

  return (
    <div
      ref={(node) => {
        containerRef.current = node;
        onContainerRef?.(node);
      }}
      data-testid="manual-workbench"
      data-search-collapsed={columnLayout.collapsed.search}
      data-map-collapsed={columnLayout.collapsed.map}
      data-results-collapsed={columnLayout.collapsed.results}
      className="relative flex h-full w-full overflow-hidden bg-sp-canvas"
      style={{ width: layoutWidth }}
    >
      {/* 1. 检索栏（左）：默认 320px，可收起为 36px 竖条 */}
      <ManualConditionsPanel
        hideMultiProviderSelector={hideMultiProviderSelector}
        collapsed={columnLayout.collapsed.search}
        onCollapsedChange={(collapsed) => collapsed ? collapseColumn("search") : expandColumn("search")}
        expandedColumnCount={expandedColumnCount}
        width={columnLayout.widths.search}
        focusCondition={focusedCondition}
        conditionsChanged={conditionsChanged}
        scientificMode={scientificMode}
        onScientificModeChange={onScientificModeChange}
        onResetParams={onResetParams}
        availableSources={availableSources}
        activeResourceId={activeResourceId}
        onSelectSource={onSelectSource}
        onOpenCatalog={
          onOpenCatalog ??
          ((opts) => {
            setCatalogGeeOnly(Boolean(opts?.geeOnly) || scientificMode === "gee");
            setCatalogOpen(true);
          })
        }
        platforms={platforms}
        onPlatformsChange={onPlatformsChange}
        productType={productType}
        onProductTypeChange={onProductTypeChange}
        polarizations={polarizations}
        onPolarizationsChange={onPolarizationsChange}
        beamModes={beamModes}
        onBeamModesChange={onBeamModesChange}
        lookDirection={lookDirection}
        onLookDirectionChange={onLookDirectionChange}
        opticalDraft={opticalDraft}
        onOpticalDraftChange={onOpticalDraftChange}
        workingAoi={workingAoi}
        workingGeometry={workingGeometry}
        aoiLabel={aoiLabel}
        onReplaceAoi={onReplaceAoi}
        onPreviewAdminGeometry={setAdminPreviewGeometry}
        onImportVectorSuccess={onImportVectorSuccess}
        startDate={startDate}
        endDate={endDate}
        onStartDateChange={onStartDateChange}
        onEndDateChange={onEndDateChange}
        orbitDirection={orbitDirection}
        onOrbitDirectionChange={onOrbitDirectionChange}
        relativeOrbit={relativeOrbit}
        onRelativeOrbitChange={onRelativeOrbitChange}
        path={path}
        onPathChange={onPathChange}
        frame={frame}
        onFrameChange={onFrameChange}
        searching={searching}
        searchError={searchError}
        onTriggerSearch={handleWrappedTriggerSearch}
        onCancelSearch={onCancelSearch}
        cancellingSearch={cancellingSearch}
        activeSourceView={activeSourceView}
        demSource={demSource}
        onDemSourceChange={onDemSourceChange}
        autoConvertEllipsoid={autoConvertEllipsoid}
        onToggleAutoConvertEllipsoid={onToggleAutoConvertEllipsoid}
        orbitSourceType={orbitSourceType}
        onOrbitSourceTypeChange={onOrbitSourceTypeChange}
        localSlcPath={localSlcPath}
        onChangeLocalSlcPath={onChangeLocalSlcPath}
        onPickLocalSlcPath={onPickLocalSlcPath}
        localDemPath={localDemPath}
        onChangeLocalDemPath={onChangeLocalDemPath}
        onPickLocalDemPath={onPickLocalDemPath}
        inlineToolResult={inlineToolResult}
        onDismissToolResult={onDismissToolResult}
        onSyncGeeResults={handleSyncedGeeResults}
        onPreviewWaybackOverlay={onPreviewWaybackOverlay}
      />

      {/* 2. 地图列（中）：依托 resolveManualColumnLayout 保持常驻停靠至 920px 最小窗口，杜绝窄屏白屏 */}
      {renderSplitter("search-map")}
      {shouldMountDockedMap(columnLayout.collapsed.map) && (
        <section
          aria-label={t("mapWorkbench.map.title")}
          data-column="map"
          data-collapsed="false"
          className="relative min-w-0 flex-1 h-full flex flex-col overflow-visible bg-sp-canvas z-10"
        >
          {/* 检索栏收起时的边缘箭头展开把手：与右侧及智能体地图收起把手统一交互风格 */}
          {columnLayout.collapsed.search && (
            <button
              type="button"
              data-testid="expand-search-column-btn"
              onClick={() => expandColumn("search")}
              aria-label={t("mapWorkbench.conditions.expandSearch")}
              title={t("mapWorkbench.conditions.expandSearch")}
              className="sp-magnetic-peek-right absolute left-0 top-1/2 -translate-y-1/2 z-[1001] w-5 h-16 rounded-r-sp-md bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover shadow-md border-y border-r border-sp-line flex flex-col items-center justify-center cursor-pointer transition-colors group"
            >
              <ChevronRight className="w-3.5 h-3.5 sp-peek-arrow" />
            </button>
          )}

          {/* 结果栏收起时的右侧边缘箭头展开把手：统一 Agent 地图收起把手的侧边吸附风格并保留 z-[1001] */}
          {columnLayout.collapsed.results && (
            <button
              type="button"
              data-testid="expand-results-column-btn"
              onClick={() => expandColumn("results")}
              aria-label={t("mapWorkbench.results.expand")}
              title={
                summary.count > 0
                  ? `${t("mapWorkbench.results.expand")} (${summary.count})`
                  : t("mapWorkbench.results.expand")
              }
              className="sp-magnetic-peek-left absolute right-0 top-1/2 -translate-y-1/2 z-[1001] w-5 h-16 rounded-l-sp-md bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover shadow-md border-y border-l border-sp-line flex flex-col items-center justify-center cursor-pointer transition-colors group"
            >
              <ChevronLeft className="w-3.5 h-3.5 sp-peek-arrow" />
            </button>
          )}

          <div className="flex-1 min-h-0 relative overflow-hidden">
            {renderWorkbenchMap()}
          </div>
        </section>
      )}
      {renderSplitter("map-results")}

      {/* 4. 地图覆盖层（窄屏浮层模式） */}
      {panelMode === "overlay" && (
        <div
          role="dialog"
          aria-label={t("mapWorkbench.map.title")}
          className="absolute right-0 top-0 bottom-0 z-30 flex flex-col w-[min(420px,calc(100%-52px))] h-full bg-sp-surface shadow-modal border-l border-sp-line"
        >
          <button
            type="button"
            onClick={() => {
              setOverlayOpen(false);
              mapButtonRef.current?.focus();
            }}
            aria-label={t("mapWorkbench.map.close")}
            title={t("mapWorkbench.map.close")}
            className="absolute top-3 right-3 z-[600] w-8 h-8 rounded-full bg-sp-surface/90 hover:bg-sp-surface border border-sp-line shadow-card flex items-center justify-center text-sp-text-secondary hover:text-sp-text cursor-pointer transition-colors"
          >
            <X className="w-4 h-4" />
          </button>

          <div className="flex-1 min-h-0 relative overflow-hidden">
            {renderWorkbenchMap()}
          </div>
        </div>
      )}

      {/* 3. 结果列（右） */}
      {!columnLayout.collapsed.results && (
      <section
        aria-label={t("mapWorkbench.results.title")}
        data-column="results"
        data-collapsed="false"
        className="min-w-0 shrink-0 h-full flex flex-col bg-sp-canvas overflow-hidden min-h-0"
        style={{ width: columnLayout.widths.results }}
      >
        {/* 顶部统计与操作条 (.sumbar — EOStage 42px 紧凑工具栏，窄屏允许温和换行不裁切核心操作) */}
        <div className="min-h-[42px] py-1.5 shrink-0 flex flex-wrap items-center justify-between gap-2 px-2.5 xl:px-3.5 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] select-none min-w-0">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 min-w-0">
            {activeResourceId === "atmospheric.gacos.ztd.v1" ? (
              <span className="text-[13px] font-semibold text-sp-text tracking-tight whitespace-nowrap shrink-0">
                {L.gacos.batchPreviewHeading}
              </span>
            ) : (
              <>
            <span className="text-[13px] font-semibold text-sp-text tabular-nums tracking-tight whitespace-nowrap shrink-0">
              {t("mapWorkbench.results.sceneCount", { total: counts.headline })}
            </span>
            {counts.loaded && (
              <span data-testid="result-loaded-count" className="text-xs text-sp-text-muted tabular-nums whitespace-nowrap shrink-0">
                {counts.loaded.total === null
                  ? t("mapWorkbench.results.loadedSoFar", { loaded: counts.loaded.loaded })
                  : t("mapWorkbench.results.loadedOfTotal", { loaded: counts.loaded.loaded, total: counts.loaded.total })}
              </span>
            )}
            {possiblyIncomplete && (
              <span data-testid="result-possibly-incomplete" className="text-xs font-medium text-sp-warning-text tabular-nums whitespace-nowrap shrink-0">
                {possiblyIncompleteLabel(totalCount)}
              </span>
            )}
            <span className="text-xs text-sp-text-secondary tabular-nums whitespace-nowrap shrink-0">
              {summary.summaryText}
            </span>
            {(checkedSceneIds.size > 0 || allMatchingSelected) && onClearChecked && (
              <button type="button" onClick={onClearChecked} className="text-xs text-sp-text-secondary hover:text-sp-text underline whitespace-nowrap shrink-0 cursor-pointer">
                {t("mapWorkbench.results.clearChecked")}
              </button>
            )}
            {onClearResults && scenes.length > 0 && !isSearching && (
              <button
                type="button"
                data-testid="manual-clear-results-btn"
                onClick={onClearResults}
                aria-label={L.resultsExtra.clearResultsAria}
                className="text-xs text-sp-text-muted hover:text-sp-text hover:underline whitespace-nowrap shrink-0 cursor-pointer"
              >
                {L.resultsExtra.clearBtn}
              </button>
            )}
              </>
            )}
          </div>

          <div className="flex items-center gap-1 shrink-0">
            <button
              type="button"
              onClick={() => collapseColumn("results")}
              disabled={expandedColumnCount === 1}
              aria-label={t("mapWorkbench.results.collapse")}
              title={expandedColumnCount === 1 ? t("mapWorkbench.conditions.keepOneColumn") : t("mapWorkbench.results.collapse")}
              className="w-7 h-7 rounded-sp-sm flex items-center justify-center text-sp-text-muted hover:text-sp-text hover:bg-sp-hover disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer shrink-0"
            >
              <PanelRight className="h-[14px] w-[14px] stroke-[1.75]" />
            </button>
            {showMapButton(panelMode) && (
              <button
                ref={mapButtonRef}
                type="button"
                onClick={() => {
                  if (panelMode === "collapsed") {
                    expandColumn("map");
                  } else if (panelMode === "hidden") {
                    setOverlayOpen(true);
                  } else {
                    collapseColumn("map");
                  }
                }}
                aria-label={t("mapWorkbench.map.title")}
                title={t("mapWorkbench.map.title")}
                className="h-8 px-2.5 rounded-sp-sm bg-sp-surface text-sp-text text-xs font-semibold border border-sp-line hover:bg-sp-hover shadow-card inline-flex items-center gap-1.5 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
              >
                <MapIcon className="w-3.5 h-3.5 text-sp-text-secondary shrink-0" />
                <span>{t("mapWorkbench.map.title")}</span>
              </button>
            )}
            {isNisarSource && onOpenNisarSubsetWorkspace && (
              <button
                type="button"
                data-testid="btn-nisar-subset-download"
                onClick={onOpenNisarSubsetWorkspace}
                disabled={summary.count === 0 || nisarSubsetBusy || Boolean(nisarSubsetDisabledReason)}
                title={
                  nisarSubsetDisabledReason
                    || (summary.count > 0
                      ? L.sar.subsetTooltipReady
                      : L.sar.subsetTooltipEmpty)
                }
                className="h-8 justify-center px-2.5 xl:min-w-[116px] xl:px-3 rounded-sp-sm border border-sp-text bg-sp-surface text-sp-text text-xs font-semibold hover:bg-sp-hover disabled:opacity-50 disabled:cursor-not-allowed shadow-card inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
              >
                {showNisarSubsetSpinner ? (
                  <Loader2 className="w-3.5 h-3.5 shrink-0 animate-spin text-sp-text-secondary" />
                ) : (
                  <Scissors className="-rotate-45 w-3.5 h-3.5 shrink-0 text-sp-text-secondary" />
                )}
                <span>{L.sar.subsetBtnLabel}</span>
              </button>
            )}
            {(activeSourceFamily === "optical" || scientificMode === "optical") &&
              canBatchOpticalAoiClip &&
              onOpenBatchOpticalAoiClip && (
                <button
                  type="button"
                  data-testid="btn-optical-aoi-clip-batch"
                  onClick={onOpenBatchOpticalAoiClip}
                  disabled={Boolean(opticalAoiClipDisabledReason)}
                  title={
                    opticalAoiClipDisabledReason
                    || t("panels.opticalAoiClip.actionBatchTitle")
                  }
                  className="h-8 px-2.5 rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text text-xs font-semibold hover:bg-sp-hover disabled:opacity-50 disabled:cursor-not-allowed shadow-card inline-flex items-center gap-1 transition-colors cursor-pointer shrink-0 whitespace-nowrap"
                >
                  <Scissors className="-rotate-45 w-3.5 h-3.5 text-sp-text-secondary shrink-0" />
                  <span>{t("panels.opticalAoiClip.actionBatch")}</span>
                </button>
              )}
            {activeResourceId !== "atmospheric.gacos.ztd.v1" && (
            <button
              type="button"
              onClick={onDownloadChecked}
              aria-busy={Boolean(downloadBusy || batchCreating)}
              disabled={summary.count === 0 || downloadBusy || batchCreating || Boolean(downloadDisabledReason) || isSearchOnly}
              title={
                isSearchOnly
                  ? t("mapWorkbench.results.searchOnly")
                  : downloadDisabledReason ||
                    (batchCreating
                      ? t("mapWorkbench.results.confirmingAccount")
                      : summary.count > 0
                      ? t("mapWorkbench.results.addCheckedTitle")
                      : t("mapWorkbench.results.selectScenesTitle"))
              }
              className="eo-tactile-primary h-8 justify-center px-2.5 xl:min-w-[116px] xl:px-3 rounded-sp-sm bg-sp-accent-fill text-white text-xs font-semibold hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed shadow-card inline-flex items-center gap-1 transition-opacity cursor-pointer shrink-0 whitespace-nowrap"
            >
              {showBatchCreatingSpinner ? (
                <Loader2 className="w-3.5 h-3.5 shrink-0 animate-spin text-white" />
              ) : (
                <Download className="w-3.5 h-3.5 shrink-0" />
              )}
              <span>
                {batchCreating
                  ? t("mapWorkbench.results.confirmingAccount")
                  : isNisarSource
                  ? L.sar.completeDownloadBtn
                  : t("mapWorkbench.results.addToQueue")}
              </span>
            </button>
            )}
          </div>
        </div>

        {counts.overCeiling && (
          <>
            <div
              data-testid="result-over-ceiling"
              className="shrink-0 flex items-center gap-1.5 px-4 py-2 text-xs bg-sp-warning-bg text-sp-warning-text"
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
              <span>{t("mapWorkbench.results.overCeiling", { total: counts.overCeiling.total, limit: counts.overCeiling.limit })}</span>
            </div>
            {activeSourceFamily === "sar" && isSentinel1 && (
              <div
                data-testid="sar-over-ceiling-sort-note"
                className="shrink-0 flex items-center gap-1.5 px-4 pb-2 pt-0 text-xs bg-sp-warning-bg text-sp-warning-text"
              >
                <Info className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
                <span>{t("mapWorkbench.results.overCeilingNewestOrderNote")}</span>
              </div>
            )}
          </>
        )}

        {!counts.overCeiling && activeSourceFamily === "optical" && counts.headline >= 2000 && !isSearching && (
          <div
            data-testid="result-optical-cap-notice"
            className="shrink-0 flex items-center gap-1.5 px-3.5 py-1.5 text-xs bg-sp-surface-muted text-sp-text-secondary border-b border-sp-line"
          >
            <Info className="h-3.5 w-3.5 shrink-0 text-sp-accent" />
            <span>{L.sar.capTruncatedNotice}</span>
          </div>
        )}

        {conditionTags.length > 0 && (
          <div className="shrink-0 px-3 py-2 bg-sp-surface-muted" aria-label={t("mapWorkbench.results.currentConditions")}>
            <div className="flex flex-wrap items-center gap-1.5 py-0.5">
              <span className="shrink-0 text-xs text-sp-text-secondary">{t("mapWorkbench.results.currentConditions")}</span>
              {conditionTags.map((item) => (
                <span
                  key={item.id}
                  data-condition-id={item.id}
                  data-condition-value={item.rawValue ?? item.text}
                  className="inline-flex h-6 shrink-0 items-center rounded-sp-sm bg-sp-selected text-xs text-sp-text-secondary"
                >
                  <button
                    type="button"
                    onClick={() => focusCondition(item.group, item.id)}
                    title={`${item.label}：${item.text}`}
                    className="h-full max-w-[200px] truncate rounded-l-sp-sm px-2 hover:text-sp-text focus-visible:outline-1 focus-visible:outline-sp-accent"
                  >
                    {item.text}
                  </button>
                  {item.removable && (
                    <button
                      type="button"
                      aria-label={t("mapWorkbench.results.removeCondition", { label: item.label })}
                      title={t("mapWorkbench.results.removeCondition", { label: item.label })}
                      onClick={() => removeCondition(item.id)}
                      className="flex h-full w-6 items-center justify-center rounded-r-sp-sm text-sp-text-muted hover:text-sp-text focus-visible:outline-1 focus-visible:outline-sp-accent"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  )}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* 排序表头 (.sorthead) */}
        {activeResourceId !== "atmospheric.gacos.ztd.v1" &&
          !((scientificMode === "dem" || activeSourceFamily === "dem") && scenes.length === 0) && (
        <div className="min-h-9 py-1 shrink-0 flex flex-wrap items-center gap-1.5 px-3 bg-sp-surface-muted border-b border-sp-line text-xs text-sp-text-secondary select-none">
          {/* 三态全选复选框 */}
          <button
            type="button"
            onClick={onToggleCheckAll}
            aria-checked={checkState === "some" ? "mixed" : checkState === "all" ? "true" : "false"}
            aria-label={t("mapWorkbench.results.selectAllLoaded")}
            aria-description={t("mapWorkbench.results.selectAllLoadedDescription")}
            title={t("mapWorkbench.results.selectAllLoadedTitle")}
            className={cn(
              "w-5 h-5 rounded-sp-sm flex items-center justify-center transition-colors duration-fast ease-sp shrink-0 cursor-pointer",
              checkState !== "none"
                ? "bg-sp-accent-strong text-white"
                : "bg-sp-surface text-transparent ring-1 ring-sp-line hover:ring-sp-accent"
            )}
          >
            {checkState === "all" && <Check className="w-3.5 h-3.5 stroke-[2.5]" />}
            {checkState === "some" && <Minus className="w-3.5 h-3.5 stroke-[2.5]" />}
          </button>

          {/* #22 B slice 5 (Owner 11-03, option A): arrived rows can be
              checked one by one; select-all over every match waits for the
              batch to finish loading. */}
          {paginated.items.length > 0 && (
            <button
              type="button"
              data-testid="select-current-page"
              disabled={pageSelectable.length === 0}
              aria-pressed={pageCheckState === "all"}
              title={t("mapWorkbench.results.selectCurrentPageTitle")}
              onClick={() => {
                if (onToggleSelectCurrentPage) {
                  onToggleSelectCurrentPage(pageSelectable);
                } else {
                  const shouldSelect = pageCheckState !== "all";
                  for (const id of pageSelectable) {
                    const checked = isRowCheckedInScope(id, rowStates?.get(id), checkedSceneIds, allMatchState);
                    if (shouldSelect !== checked) {
                      onToggleCheckScene(id);
                    }
                  }
                }
              }}
              className={cn(
                "h-6 px-1.5 rounded-sp-sm text-xs whitespace-nowrap transition-colors duration-fast ease-sp shrink-0 cursor-pointer disabled:cursor-not-allowed disabled:opacity-50",
                pageCheckState === "all"
                  ? "bg-sp-selected text-sp-text font-medium ring-1 ring-sp-line"
                  : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface",
              )}
            >
              {pageCheckState === "all"
                ? t("mapWorkbench.results.unselectCurrentPage")
                : t("mapWorkbench.results.selectCurrentPage")}
            </button>
          )}
          {!selectAllMatchingAvailable(batchDelivery) && counts.headline >= 250 && (
            <span title={t("mapWorkbench.results.selectAllMatchingWaiting")} className="shrink-0">
              <button
                type="button"
                disabled
                aria-description={t("mapWorkbench.results.selectAllMatchingWaiting")}
                className="h-6 px-1.5 rounded-sp-sm text-xs whitespace-nowrap text-sp-text-muted disabled:cursor-not-allowed disabled:opacity-60"
              >
                {t("mapWorkbench.results.selectAllMatching")}
              </button>
            </span>
          )}

          {selectAllMatchingAvailable(batchDelivery) && !possiblyIncomplete && (counts.headline >= 250 || Boolean(allMatchingSelected)) && !counts.overCeiling && onToggleSelectAllMatching && (
            <button
              type="button"
              data-testid="select-all-matching"
              aria-pressed={Boolean(allMatchingSelected)}
              title={t("mapWorkbench.results.selectAllMatching")}
              onClick={onToggleSelectAllMatching}
              className={cn(
                "h-6 px-1.5 rounded-sp-sm text-xs whitespace-nowrap transition-colors duration-fast ease-sp shrink-0 cursor-pointer",
                allMatchingSelected
                  ? "bg-sp-accent-strong text-white font-semibold shadow-card"
                  : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface ring-1 ring-sp-line",
              )}
            >
              {t("mapWorkbench.results.selectAllMatching")}
            </button>
          )}

          <span className="text-xs text-sp-text-secondary shrink-0">{t("mapWorkbench.results.sort")}</span>

          <div className="flex flex-wrap items-center gap-0.5 min-w-0">
            {scientificMode !== "gee" &&
              activeResourceId !== "atmospheric.gacos.ztd.v1" &&
              activeResourceId !== "auxiliary.gacos.ztd.v1" && (
              <button
                type="button"
                data-testid="toggle-sar-stack-grouping"
                aria-pressed={groupByStack}
                onClick={() => setGroupByStack((prev) => !prev)}
                title={t("mapWorkbench.results.groupByFrameTitle")}
                className={cn(
                  "h-6 px-1.5 inline-flex items-center gap-1 rounded-sp-sm text-xs whitespace-nowrap transition-colors duration-fast ease-sp cursor-pointer shrink-0 mr-0.5",
                  groupByStack
                    ? "bg-sp-selected text-sp-text font-semibold ring-1 ring-sp-text/40"
                    : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface"
                )}
              >
                <Layers className="w-3 h-3 shrink-0" />
                <span>{t("mapWorkbench.results.groupByFrame")}</span>
              </button>
            )}
            {availableSortKeys.map((item) => {
              const isActive = effectiveSortKey === item.key;
              return (
                <button
                  key={item.key}
                  type="button"
                  onClick={() => setSortState((prev) => nextSort(prev, item.key))}
                  className={cn(
                    "h-6 px-1.5 inline-flex items-center gap-0.5 rounded-sp-sm text-xs whitespace-nowrap transition-colors duration-fast ease-sp cursor-pointer shrink-0",
                    isActive
                      ? "bg-sp-surface text-sp-text font-semibold shadow-card ring-1 ring-sp-line"
                      : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface"
                  )}
                >
                  <span>{item.label}</span>
                  {isActive && (
                    sortState.dir === "desc" ? (
                      <ArrowDown className="w-3.5 h-3.5 text-sp-text" aria-label={t("mapWorkbench.results.descending")} />
                    ) : (
                      <ArrowUp className="w-3.5 h-3.5 text-sp-text" aria-label={t("mapWorkbench.results.ascending")} />
                    )
                  )}
                </button>
              );
            })}
          </div>

          <span
            className={cn(
              "ml-auto text-xs text-sp-text-secondary whitespace-nowrap",
              columnLayout.widths.results < 380 && "hidden"
            )}
          >
            {t("mapWorkbench.results.taskStatus")}
          </span>
        </div>
        )}

        {/* B3(a): 任务创建反馈就地内联通知条 */}
        {taskOutcomeNotice && (
          <div
            className={cn(
              "shrink-0 flex items-center justify-between px-4 py-2 text-xs border-b border-sp-line",
              taskOutcomeNotice.tone === "danger"
                ? "bg-sp-danger-bg text-sp-danger-text"
                : "bg-sp-surface-muted text-sp-text-secondary"
            )}
          >
            <div className="flex items-center gap-1.5 min-w-0">
              {taskOutcomeNotice.tone === "danger" ? (
                <AlertTriangle className="w-3.5 h-3.5 text-sp-status-danger shrink-0" />
              ) : (
                <Info className="w-3.5 h-3.5 text-sp-text-secondary shrink-0" />
              )}
              <span className="min-w-0 break-words">{taskOutcomeNotice.text}</span>
            </div>
            <button
              type="button"
              onClick={onDismissTaskOutcomeNotice}
              aria-label={t("mapWorkbench.results.close")}
              title={t("mapWorkbench.results.close")}
              className="w-5 h-5 flex items-center justify-center rounded-sp-sm hover:opacity-75 transition-opacity cursor-pointer shrink-0 ml-2"
            >
              <X className="w-3.5 h-3.5" />
            </button>
          </div>
        )}

        {/* 检索状态横幅 */}
        {isSearching && (
          <div
            data-testid="search-status-banner"
            title={
              deliveryLoading
                ? t("mapWorkbench.results.loadingRemaining")
                : hasPreviousBatch
                ? t("mapWorkbench.results.previousBatchSearching")
                : t("mapWorkbench.results.searching")
            }
            className="flex h-8 shrink-0 items-center justify-between gap-2 bg-sp-surface-muted px-3 text-xs text-sp-text border-b border-sp-line font-medium overflow-hidden"
          >
            <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
              <span className="relative inline-flex h-4 w-4 shrink-0 items-center justify-center" aria-hidden="true">
                {showSearchProgressSpinner && <Loader2 className="h-3.5 w-3.5 animate-spin text-sp-accent" />}
              </span>
              <span className="truncate whitespace-nowrap text-xs text-sp-text">
                {deliveryLoading
                  ? t("mapWorkbench.results.loadingRemaining")
                  : searchProgress?.stage === "planning"
                  ? t("mapWorkbench.results.stagePlanning")
                  : searchProgress?.stage === "requesting"
                  ? t("mapWorkbench.results.stageRequesting")
                  : searchProgress?.stage === "filtering"
                  ? t("mapWorkbench.results.stageFiltering")
                  : searchProgress?.stage === "freezing"
                  ? t("mapWorkbench.results.stageFreezing")
                  : t("mapWorkbench.results.searching")}
              </span>
              <span className="shrink-0 whitespace-nowrap rounded-sp-sm bg-sp-surface px-1.5 py-0.5 font-mono text-xs text-sp-text-secondary border border-sp-line">
                {`${searchProgress?.items_kept ?? 0} / ${searchProgress?.items_cap ?? 0}`}
              </span>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {hasPreviousBatch && (
                <span
                  title={t("mapWorkbench.results.retainedPreviousBatch", { total: totalCount })}
                  className="hidden xl:inline-block shrink-0 whitespace-nowrap font-mono text-xs text-sp-text-muted"
                >
                  {t("mapWorkbench.results.retainedPreviousBatch", { total: totalCount })}
                </span>
              )}
              {onCancelSearch && (
                <button
                  type="button"
                  onClick={onCancelSearch}
                  disabled={cancellingSearch}
                  aria-label={t("mapWorkbench.results.cancelSearchLabel")}
                  title={t("mapWorkbench.results.cancelSearchLabel")}
                  className="h-6 shrink-0 whitespace-nowrap px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover disabled:opacity-60 disabled:cursor-wait cursor-pointer"
                >
                  <X className="h-3.5 w-3.5 shrink-0" />
                  <span>{cancellingSearch ? t("mapWorkbench.results.cancellingSearch") : t("mapWorkbench.results.cancelSearch")}</span>
                </button>
              )}
            </div>
          </div>
        )}

        {/* 失败保留上一批横幅 */}
        {hasLastErrorWithBatch && !isSearching && (
          <div
            title={t("mapWorkbench.results.latestSearchFailed", { reason: preferCodeExplanation(activeSourceView?.last_error?.message_zh, activeSourceView?.last_error?.code) || t("mapWorkbench.results.noReasonReturned") })}
            className="flex h-8 shrink-0 items-center justify-between gap-2 bg-sp-warning-bg px-3 text-xs text-sp-warning-text border-b border-sp-line font-medium overflow-hidden"
          >
            <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden">
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
              <span className="truncate whitespace-nowrap">
                {t("mapWorkbench.results.latestSearchFailed", { reason: preferCodeExplanation(activeSourceView?.last_error?.message_zh, activeSourceView?.last_error?.code) || t("mapWorkbench.results.noReasonReturned") })}
              </span>
            </div>
            <span className="shrink-0 whitespace-nowrap font-mono text-xs text-sp-warning-text/80">
              {t("mapWorkbench.results.continueBrowsing")}
            </span>
          </div>
        )}

        {/* 旧批次（超过 24 小时）过期提醒与一键强制刷新 */}
        {showStaleBatchBanner && (
          <div
            data-testid="stale-batch-banner"
            role="status"
            className="flex h-8 shrink-0 items-center justify-between gap-2 bg-sp-warning-bg/70 px-3 text-xs text-sp-warning-text border-b border-sp-line font-medium overflow-hidden"
          >
            <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden">
              <Clock className="h-3.5 w-3.5 shrink-0 text-sp-status-warning" />
              <span className="truncate whitespace-nowrap">
                {t("mapWorkbench.results.staleBatchNotice", { age: staleBatchAgeLabel })}
              </span>
            </div>
            <button
              type="button"
              data-testid="stale-batch-refresh-btn"
              disabled={!canRefreshStaleBatch}
              title={!canRefreshStaleBatch ? t("mapWorkbench.desktop.aoiRequired") : t("mapWorkbench.results.refreshLatestResults")}
              onClick={() => (onRefreshSearch ?? handleWrappedTriggerSearch)()}
              className="h-6 shrink-0 whitespace-nowrap px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-warning-text hover:bg-sp-hover disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer"
            >
              <RefreshCw className="h-3 w-3 shrink-0" />
              <span>{t("mapWorkbench.results.refreshLatestResults")}</span>
            </button>
          </div>
        )}

        {/* 结果行列表或空状态，底部留白 pb-20 确保最后一行不被浮动助手遮挡 */}
        {activeResourceId === "atmospheric.gacos.ztd.v1" ? (
          <GacosBatchPreviewPanel
            workingAoi={workingAoi}
            aoiLabel={aoiLabel}
            startDate={startDate}
            endDate={endDate}
            batchStatuses={gacosBatchStatuses}
            selectedBatchIndices={selectedGacosBatchIndices}
            onToggleBatchIndex={onToggleGacosBatchIndex}
            onSelectAllBatches={onSelectAllGacosBatches}
            onRetryBatch={onRetryGacosBatch}
            submitting={gacosSubmitting}
          />
        ) : sourceViewLoading ? (
          <div role="status" aria-busy="true" aria-label={t("mapWorkbench.desktop.sourceViewLoading")} className="flex-1 overflow-y-auto p-2">
            {showSourceViewSkeleton ? (
              <CatalogSkeletonRows rows={6} />
            ) : (
              <div className="h-full min-h-[280px]" />
            )}
          </div>
        ) : sourceViewError ? (
          <div className="flex-1 flex items-center justify-center">
            <EmptyState
              kind="failed"
              size="page"
              visual={<EmptyStateIcon name="alert" size={48} />}
              title={L.resultsExtra.listLoadFailedTitle}
              description={L.resultsExtra.listLoadFailedDesc}
              primaryAction={
                onRetrySourceView
                  ? {
                      label: t("mapWorkbench.results.retryLoadMore"),
                      onClick: onRetrySourceView,
                      loading: sourceViewLoading,
                    }
                  : undefined
              }
            />
          </div>
        ) : counts.headline === 0 && isSearching ? (
          showSearchProgressSpinner ? (
            <div role="status" aria-busy="true" aria-label={t("mapWorkbench.results.searching")} className="flex-1 overflow-y-auto p-2">
              <CatalogSkeletonRows rows={6} />
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center">
              <EmptyState
                kind="first-use"
                size="page"
                visual={<EmptyStateWireframe variant="scene-search" />}
                title={L.resultsExtra.searchingTitle}
              />
            </div>
          )
        ) : counts.headline === 0 && !isSearching && (searchError || activeSourceView?.last_error) ? (
          <div className="flex-1 flex items-center justify-center">
            <EmptyState
              kind="failed"
              size="page"
              visual={<EmptyStateIcon name="alert" size={48} />}
              title={L.resultsExtra.searchIncompleteTitle}
              description={
                preferCodeExplanation(
                  activeSourceView?.last_error?.message_zh,
                  activeSourceView?.last_error?.code
                ) || L.resultsExtra.searchIncompleteFallback
              }
              primaryAction={{
                label: L.resultsExtra.retryBtn,
                onClick: onTriggerSearch,
                loading: isSearching,
              }}
            />
          </div>
        ) : counts.headline === 0 && !isSearching ? (
          activeSourceView?.batch_id ? (
            <div className="flex-1 flex items-center justify-center">
              <EmptyState
                kind="no-results"
                size="page"
                visual={<EmptyStateWireframe variant="no-results" />}
                title={L.resultsExtra.noMatchTitle}
                description={L.resultsExtra.noMatchDesc}
                primaryAction={
                  onResetParams
                    ? { label: L.resultsExtra.resetFilterBtn, onClick: onResetParams }
                    : undefined
                }
              />
            </div>
          ) : (() => {
            const isDemPlanningMode = scientificMode === "dem" || activeSourceFamily === "dem";
            return (
              <div className="flex-1 flex items-center justify-center">
                <EmptyState
                  kind="first-use"
                  size="page"
                  visual={<EmptyStateWireframe variant="scene-search" showAccentAction />}
                  title={
                    workingAoi
                      ? aoiLabel
                        ? isDemPlanningMode
                          ? t("mapWorkbench.results.locatedAoi", { label: aoiLabel })
                          : L.resultsExtra.emptyLocatedAoi.replace("{label}", aoiLabel)
                        : isDemPlanningMode
                          ? t("mapWorkbench.results.selectedAoi")
                          : L.resultsExtra.emptySelectedAoi
                      : isDemPlanningMode
                        ? t("mapWorkbench.results.demEmptyTitle")
                        : L.resultsExtra.emptySearchTitle
                  }
                  description={
                    isDemPlanningMode
                      ? workingAoi
                        ? t("mapWorkbench.results.demEmptyWithAoiDesc")
                        : t("mapWorkbench.results.demEmptyNoAoiDesc")
                      : workingAoi
                        ? L.resultsExtra.emptySearchWithAoiDesc
                        : L.resultsExtra.emptySearchNoAoiDesc
                  }
                  primaryAction={
                    workingAoi
                      ? {
                          label: isDemPlanningMode ? t("mapWorkbench.conditions.planDem") : L.resultsExtra.startSearchBtn,
                          onClick: onTriggerSearch,
                        }
                      : {
                          label: L.resultsExtra.drawAoiBtn,
                          onClick: () => {
                            onDrawModeChange("rect");
                            onDrawActiveChange(true);
                          },
                        }
                  }
                />
              </div>
            );
          })()
        ) : (
          <div ref={resultsScrollRef} data-testid="results-scroll-container" className="flex-1 min-h-0 overflow-y-auto pb-20">
            <div className="flex flex-col">
              {groupedSarStacks ? (
                groupedSarStacks.map((stack) => {
                  const stackIds = stack.items.map((s) => s.scene_id);
                  const groupCollapsed = collapsedStackKeys.has(stack.key);
                  const allStackChecked =
                    stackIds.length > 0 &&
                    stackIds.every((id) =>
                      isRowCheckedInScope(id, rowStates?.get(id), checkedSceneIds, allMatchState)
                    );
                  return (
                    <div key={stack.key} className="border-b border-sp-line/60">
                      <div
                        data-testid={`sar-stack-group-${stack.key}`}
                        className="sticky top-0 z-10 flex items-center justify-between gap-2 bg-sp-surface-muted/95 backdrop-blur-xs px-3 py-1.5 border-b border-sp-line text-xs"
                      >
                        <button
                          type="button"
                          onClick={() => toggleStackCollapse(stack.key)}
                          aria-expanded={!groupCollapsed}
                          className="flex items-center gap-1.5 min-w-0 flex-1 text-left cursor-pointer select-none hover:opacity-85"
                        >
                          {groupCollapsed ? (
                            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                          ) : (
                            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                          )}
                          <Layers className="h-3.5 w-3.5 shrink-0 text-sp-accent" />
                          <span className="font-semibold text-sp-text truncate">{stack.label}</span>
                          <span className="shrink-0 rounded-sp-sm bg-sp-surface px-1.5 py-0.5 font-mono text-xs text-sp-text-secondary border border-sp-line tabular-nums">
                            {t("mapWorkbench.results.frameGroupCount", { count: stack.items.length })}
                          </span>
                        </button>
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            if (onToggleSelectCurrentPage) {
                              onToggleSelectCurrentPage(stackIds);
                            } else {
                              for (const id of stackIds) {
                                const checked = isRowCheckedInScope(
                                  id,
                                  rowStates?.get(id),
                                  checkedSceneIds,
                                  allMatchState
                                );
                                if (allStackChecked ? checked : !checked) {
                                  onToggleCheckScene(id);
                                }
                              }
                            }
                          }}
                          className="shrink-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                        >
                          {allStackChecked
                            ? t("mapWorkbench.results.unselectGroup")
                            : t("mapWorkbench.results.selectGroup")}
                        </button>
                      </div>
                      {!groupCollapsed && stack.items.map((scene, sIdx) => (
                        <CatalogResultRow
                          key={scene.scene_id}
                          scene={scene}
                          className="sp-stagger-fade-row"
                          style={{ "--row-index": Math.min(sIdx, 10) } as React.CSSProperties}
                          batchId={shownBatchId}
                          rowState={rowStates?.get(scene.scene_id)}
                          task={
                            rowStates?.get(scene.scene_id)?.task_id
                              ? tasksById?.get(rowStates.get(scene.scene_id)!.task_id!)
                              : undefined
                          }
                          isSelected={selectedSceneId === scene.scene_id}
                          isChecked={isRowCheckedInScope(
                            scene.scene_id,
                            rowStates?.get(scene.scene_id),
                            checkedSceneIds,
                            allMatchState
                          )}
                          isCreating={creatingItemIds?.has(scene.scene_id)}
                          isFailureExpanded={expandedFailureSceneIds.has(scene.scene_id)}
                          family={activeSourceFamily}
                          hideDownload={isSearchOnly}
                          onToggleFailureExpand={handleToggleFailureExpand}
                          onSelectScene={(id) => onSelectScene(id || null)}
                          onHoverChange={setHoveredSceneId}
                          onToggleCheckScene={onToggleCheckScene}
                          onRowAction={onRowAction}
                          canOpticalAoiClip={canRowOpticalAoiClip ? canRowOpticalAoiClip(scene) : false}
                          onOpticalAoiClip={onOpenRowOpticalAoiClip}
                          onPreviewScene={onPreviewGeeScene}
                          isPreviewPinned={pinnedPreviewIds.has(scene.scene_id)}
                          onTogglePinPreview={togglePinPreviewScene}
                          companionStatusText={
                            isSarCompanion && ((isSentinel1 && companionState.preciseOrbit) || companionState.dem || companionState.gacos)
                              ? [
                                  isSentinel1 && companionState.preciseOrbit
                                    ? calculateOrbitMatch([scene], activeResourceId).matchedCount > 0
                                      ? t("panels.companionDownload.orbitMatched", { count: 1 })
                                      : t("panels.companionDownload.orbitPending", { count: 1 })
                                    : null,
                                  companionState.dem ? "30m DEM" : null,
                                  companionState.gacos ? L.gacos.syncSubmitLabel : null,
                                ]
                                  .filter(Boolean)
                                  .join(" · ")
                              : undefined
                          }
                        />
                      ))}
                    </div>
                  );
                })
              ) : (
                /* scenes.map (paginated items) */
                paginated.items.map((scene, idx) => (
                  <CatalogResultRow
                    key={scene.scene_id}
                    scene={scene}
                    className="sp-stagger-fade-row"
                    style={{ "--row-index": Math.min(idx, 10) } as React.CSSProperties}
                    batchId={shownBatchId}
                    rowState={rowStates?.get(scene.scene_id)}
                    task={
                      rowStates?.get(scene.scene_id)?.task_id
                        ? tasksById?.get(rowStates.get(scene.scene_id)!.task_id!)
                        : undefined
                    }
                    isSelected={selectedSceneId === scene.scene_id}
                    isChecked={isRowCheckedInScope(scene.scene_id, rowStates?.get(scene.scene_id), checkedSceneIds, allMatchState)}
                    isCreating={creatingItemIds?.has(scene.scene_id)}
                    isFailureExpanded={expandedFailureSceneIds.has(scene.scene_id)}
                    family={activeSourceFamily}
                    hideDownload={isSearchOnly}
                    onToggleFailureExpand={handleToggleFailureExpand}
                    onSelectScene={(id) => onSelectScene(id || null)}
                    onHoverChange={setHoveredSceneId}
                    onToggleCheckScene={onToggleCheckScene}
                    onRowAction={onRowAction}
                    canOpticalAoiClip={canRowOpticalAoiClip ? canRowOpticalAoiClip(scene) : false}
                    onOpticalAoiClip={onOpenRowOpticalAoiClip}
                    onPreviewScene={onPreviewGeeScene}
                    isPreviewPinned={pinnedPreviewIds.has(scene.scene_id)}
                    onTogglePinPreview={togglePinPreviewScene}
                    companionStatusText={
                      isSarCompanion && ((isSentinel1 && companionState.preciseOrbit) || companionState.dem || companionState.gacos)
                        ? [
                            isSentinel1 && companionState.preciseOrbit
                              ? calculateOrbitMatch([scene], activeResourceId).matchedCount > 0
                                ? t("panels.companionDownload.orbitMatched", { count: 1 })
                                : t("panels.companionDownload.orbitPending", { count: 1 })
                              : null,
                            companionState.dem ? "30m DEM" : null,
                            companionState.gacos ? L.gacos.syncSubmitLabel : null,
                          ]
                            .filter(Boolean)
                            .join(" · ")
                        : undefined
                    }
                  />
                ))
              )}
              {arrival !== "ready" && !loadMoreError && (
                <div
                  data-testid="result-page-loading"
                  role="status"
                  aria-busy="true"
                  aria-label={t("mapWorkbench.results.pageLoading")}
                  className={cn(
                    "px-2",
                    arrival === "pending" ? "min-h-40 py-2" : "min-h-11 py-1",
                  )}
                >
                  {showResultPageSkeleton ? (
                    <CatalogSkeletonRows rows={arrival === "pending" ? 5 : 2} />
                  ) : (
                    <div className={arrival === "pending" ? "min-h-36" : "min-h-9"} />
                  )}
                </div>
              )}
              {(loadMoreError || (arrival === "ready" && !deliveryLoading && (hasMoreResults || loadingMore))) && (
                <div className="flex min-h-11 items-center justify-center gap-2 px-4 py-2 text-xs text-sp-text-secondary">
                  <button
                    type="button"
                    onClick={onLoadMore}
                    aria-busy={loadingMore}
                    disabled={loadingMore || !onLoadMore}
                    className="inline-flex min-w-[104px] justify-center items-center gap-1.5 rounded-sp-sm px-3 py-1.5 hover:bg-sp-hover disabled:cursor-wait disabled:opacity-60 cursor-pointer"
                  >
                    {showLoadMoreSpinner && <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" aria-hidden="true" />}
                    <span>{loadMoreError ? t("mapWorkbench.results.retryLoadMore") : t("mapWorkbench.results.loadMore")}</span>
                  </button>
                  <span className={loadMoreError ? "text-sp-danger-text" : "tabular-nums"}>
                    {loadMoreError ?? t("mapWorkbench.results.loadedCount", { loaded: scenes.length, total: totalCount })}
                  </span>
                </div>
              )}
            </div>
          </div>
        )}

        {/* 分页栏（当场景总数超过 50 时展示） */}
        {paginated.pageCount > 1 && (
          <div className="h-10 shrink-0 flex items-center justify-between px-4 border-t border-sp-line bg-sp-surface-muted text-xs text-sp-text-secondary select-none">
            <span className="tabular-nums">
              {t("mapWorkbench.results.pageCount", { page: paginated.page, pages: paginated.pageCount })}
            </span>
            <div className="flex items-center gap-1">
              <button
                type="button"
                disabled={paginated.page <= 1}
                onClick={() => setCurrentPage((p) => Math.max(1, p - 1))}
                aria-label={t("mapWorkbench.results.previousPage")}
                title={t("mapWorkbench.results.previousPage")}
                className="w-7 h-7 inline-flex items-center justify-center rounded-sp-sm text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer transition-colors"
              >
                <ChevronLeft className="w-4 h-4" />
              </button>
              <button
                type="button"
                disabled={paginated.page >= paginated.pageCount}
                onClick={() => setCurrentPage((p) => Math.min(paginated.pageCount, p + 1))}
                aria-label={t("mapWorkbench.results.nextPage")}
                title={t("mapWorkbench.results.nextPage")}
                className="w-7 h-7 inline-flex items-center justify-center rounded-sp-sm text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer transition-colors"
              >
                <ChevronRight className="w-4 h-4" />
              </button>
            </div>
          </div>
        )}
        {/* 结果栏底部操作条 (改版第 7 步) */}
        {activeResourceId !== "atmospheric.gacos.ztd.v1" && (
        <div
          data-testid="results-footer-bar"
          className="shrink-0 flex items-center justify-between gap-2 px-3 py-2 border-t border-sp-line bg-sp-card select-none"
        >
          <div className="flex items-center gap-2 min-w-0">
            <span className="text-xs text-sp-text-secondary truncate">
              {summary.count > 0 ? (
                <>
                  <span className="font-semibold text-sp-text">{summary.count}</span> {t("mapWorkbench.results.sceneCount", { total: summary.count }).replace(String(summary.count), "").trim()}
                  {companionState.preciseOrbit && isSentinel1 && (
                    <span className="text-sp-text-muted before:content-['·_']">
                      {orbitMatch.matchedCount > 0
                        ? t("panels.companionDownload.orbitMatched", { count: orbitMatch.matchedCount })
                        : t("panels.companionDownload.orbitPending", { count: orbitMatch.pendingCount })}
                    </span>
                  )}
                  {isSarCompanion && companionState.dem && (
                    <span className="text-sp-text font-medium before:content-['·_']">
                      30m DEM
                    </span>
                  )}
                  {isSarCompanion && companionState.gacos && (
                    <span className="text-sp-text font-medium before:content-['·_']">
                      {L.gacos.footerCountLabel.replace("{count}", String(gacosEstimate.requestCount))}
                    </span>
                  )}
                </>
              ) : (
                <span>{t("mapWorkbench.results.selectScenesTitle")}</span>
              )}
            </span>
          </div>

          {isSarCompanion && scientificMode !== "gee" && (
            <div className="relative flex items-center gap-2 shrink-0">
              {/* 配套下载按钮 */}
              <button
                type="button"
                data-testid="btn-companion-download"
                onClick={() => setCompanionPopoverOpen((prev) => !prev)}
                className={cn(
                  "eo-tactile-secondary h-8 px-2.5 rounded-sp-sm border border-sp-line text-xs font-medium inline-flex items-center gap-1 cursor-pointer",
                  companionPopoverOpen || (companionState.preciseOrbit && isSentinel1) || companionState.dem || companionState.gacos
                    ? "bg-sp-selected text-sp-text border-sp-text"
                    : "bg-sp-surface text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover"
                )}
              >
                <span>{companionButtonLabel}</span>
                <ChevronDown className={cn("h-3.5 w-3.5 transition-transform", companionPopoverOpen && "rotate-180")} />
              </button>

              {/* 配套下载弹层 */}
              <CompanionDownloadPopover
                isOpen={companionPopoverOpen}
                onClose={() => setCompanionPopoverOpen(false)}
                activeResourceId={activeResourceId}
                selectedScenes={checkedScenesList}
                workingAoiBbox={workingAoi}
                companionState={companionState}
                onCompanionStateChange={setCompanionState}
                onTriggerStandaloneCompanion={
                  onTriggerStandaloneCompanion
                    ? async () => {
                        await onTriggerStandaloneCompanion();
                        setCompanionPopoverOpen(false);
                      }
                    : undefined
                }
                standaloneBusy={standaloneCompanionBusy}
              />
            </div>
          )}
        </div>
        )}
      </section>
      )}

      <DataSourceCatalogModal
        isOpen={catalogOpen}
        lockedToGee={catalogGeeOnly || scientificMode === "gee"}
        onClose={() => {
          setCatalogOpen(false);
          setCatalogGeeOnly(false);
        }}
        availableSources={availableSources}
        activeResourceId={activeResourceId}
        onSelectSource={(resourceId) => {
          void Promise.resolve(onSelectSource(resourceId)).catch(() => {});
        }}
        workingBbox={workingAoi}
        onOpenAccountSettings={
          onOpenSettings
            ? () => {
                setCatalogOpen(false);
                onOpenSettings();
              }
            : undefined
        }
      />

      {/* 就地登录引导弹窗（Component 自身已是 fixed z-[2600] 模态层，避免外层 stacking context 压低层级） */}
      {inlineLoginError && onCloseInlineLogin && onSuccessInlineLogin && onOpenSettings && (
        <InplaceLoginCard
          error={inlineLoginError}
          onClose={onCloseInlineLogin}
          onSuccess={onSuccessInlineLogin}
          onOpenSettings={onOpenSettings}
        />
      )}
    </div>
  );
}
