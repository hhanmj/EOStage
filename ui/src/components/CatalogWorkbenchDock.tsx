import {
  ChevronDown,
  ChevronUp,
  Clock3,
  Database,
  Download,
  Eye,
  Files,
  Info,
  Layers3,
  Loader2,
  LockKeyhole,
  MapPinned,
  Pause,
  Play,
} from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import type { DemDockProjection, DockPreferences } from "@/lib/catalogDock";
import {
  clampDockHeight,
  scrollTopForProduct,
  virtualRowWindow,
} from "@/lib/catalogDock";
import type { CatalogResultRow, PreviewRuntimeState } from "@/lib/catalogPreview";
import { restrictedSarReason } from "@/lib/catalogPreview";
import type { CoreWorkbenchSourceAttemptProjection } from "@/lib/coreRestoreProjection";
import type { SarProduct } from "@/lib/generated/v3-contracts";
import { catalogSearchCountSummary } from "@/lib/catalogSearchCountSummary";
import { cn } from "@/lib/utils";
import { pageIndexFromScrollTop, scrollTopFromPageIndex } from "@/lib/workspaceViewState";

const CATALOG_ROW_HEIGHT = 72;

export type CatalogWorkbenchDockProps = {
  rows: readonly CatalogResultRow[];
  products: readonly SarProduct[];
  selectedProductKeys: ReadonlySet<string>;
  activeDownloadProductKeys: ReadonlySet<string>;
  pausedDownloadProductKeys: ReadonlySet<string>;
  taskProductKeys: ReadonlySet<string>;
  demTasks: readonly DemDockProjection[];
  preview: PreviewRuntimeState;
  activePreviewProductKey: string | null;
  previewProductLabel: string | null;
  previewSaving?: boolean;
  previewSavedPath?: string | null;
  filterQuery: string;
  pageIndex: number;
  pageSize: number;
  boundedPageMode?: boolean;
  totalCount?: number | null;
  checkedCount?: number | null;
  searchCounts?: {
    targetCount: number | null;
    providerMatchedCount: number | null;
    providerReturnedCount: number | null;
    frozenCandidateCount: number;
    truncated: boolean;
  } | null;
  sourceAttempts?: readonly CoreWorkbenchSourceAttemptProjection[];
  pageBusy?: boolean;
  searchPending?: boolean;
  searchPendingLabel?: string;
  revealProductKey?: string | null;
  revealGeneration?: number;
  preferences: DockPreferences;
  onPreferencesChange: (next: DockPreferences) => void;
  onFilterQueryChange: (value: string) => void;
  onPageIndexChange: (pageIndex: number) => void;
  onActivateProduct: (productKey: string) => void;
  onToggleProduct: (productKey: string, selected: boolean) => void;
  onShowProductInfo: (productKey: string, anchor: DOMRect) => void;
  onHideProductInfo: () => void;
  onCopyProductInfo: (productKey: string) => void;
  onPreviewProduct?: (productKey: string) => void;
  assetDetailProductKeys?: ReadonlySet<string>;
  assetDetailsLoadingProductKey?: string | null;
  onAssetDetailsProduct?: (productKey: string) => void;
  onVisibleProductKeysChange?: (productKeys: readonly string[]) => void;
  onPauseProduct: (productKey: string) => void;
  onResumeProduct: (productKey: string) => void;
  onDownloadProduct: (productKey: string) => void;
  onSelectAll: () => void;
  onClearSelection: () => void;
  onDownloadSelected: () => void;
  showDownloadSelected?: boolean;
  rowDownloadLabel?: string;
  acquisitionOptions?: ReactNode;
  onClosePreview: () => void;
  onCancelPreview: () => void;
  onRetryPreview?: () => void;
  onSavePreview: () => void;
  onOpenPreviewSaveDirectory: () => void;
  onPauseSelected: () => void;
  onResumeSelected: () => void;
  selectedPausableCount?: number;
  selectedResumableCount?: number;
  showSelectedTaskActions?: boolean;
  taskPaused?: boolean;
  resumeSelectedDisabledReason?: string | null;
  rowDownloadDisabledReason?: string | null;
  downloadBusy?: boolean;
  downloadDisabledReason?: string | null;
};

function stopPropagation(event: { stopPropagation: () => void }) {
  event.stopPropagation();
}

function rowAction(
  row: CatalogResultRow,
  downloading: boolean,
  paused: boolean,
  taskMember: boolean,
  props: Pick<
    CatalogWorkbenchDockProps,
    | "taskPaused"
    | "downloadBusy"
    | "rowDownloadDisabledReason"
    | "onPauseProduct"
    | "onResumeProduct"
    | "onDownloadProduct"
    | "rowDownloadLabel"
  >,
) {
  const taskPaused = Boolean(props.taskPaused);
  if (downloading) {
    return (
      <Button
        size="sm"
        variant="outline"
        className="h-8 shrink-0 px-2"
        onClick={(event) => {
          event.stopPropagation();
          props.onPauseProduct(row.provider_product_key);
        }}
        onKeyDown={stopPropagation}
      >
        <Pause className="h-3.5 w-3.5" />
        暂停
      </Button>
    );
  }
  if (paused && !taskPaused) {
    return (
      <Button
        size="sm"
        variant="outline"
        className="h-8 shrink-0 px-2"
        onClick={(event) => {
          event.stopPropagation();
          props.onResumeProduct(row.provider_product_key);
        }}
        onKeyDown={stopPropagation}
      >
        <Play className="h-3.5 w-3.5" />
        继续
      </Button>
    );
  }
  if (taskMember) {
    return (
      <Button
        size="sm"
        variant="outline"
        className="h-8 shrink-0 px-2"
        disabled
        title={taskPaused ? "整个任务已暂停，请在任务队列中继续" : "该产品已在当前任务中"}
      >
        <Clock3 className="h-3.5 w-3.5" />
        {taskPaused ? "任务暂停" : "任务中"}
      </Button>
    );
  }
  if (row.download_restricted) {
    return (
      <Button
        size="sm"
        variant="outline"
        className="h-8 shrink-0 px-2"
        disabled
        title={restrictedSarReason(row.dataset)}
      >
        <LockKeyhole className="h-3.5 w-3.5" />
        受限数据
      </Button>
    );
  }
  return (
    <Button
      size="sm"
      variant="outline"
      className="h-8 shrink-0 px-2"
      disabled={props.downloadBusy || Boolean(props.rowDownloadDisabledReason)}
      title={props.rowDownloadDisabledReason || (props.rowDownloadLabel === "完整产品" ? "下载该完整产品" : "只下载该产品")}
      onClick={(event) => {
        event.stopPropagation();
        props.onDownloadProduct(row.provider_product_key);
      }}
      onKeyDown={stopPropagation}
    >
      <Download className="h-3.5 w-3.5" />
      {props.rowDownloadLabel ?? "下载"}
    </Button>
  );
}

function sourceProviderLabel(providerId: string): string {
  switch (providerId) {
    case "microsoft.planetary-computer": return "Microsoft Planetary Computer";
    case "element84.earth-search": return "Element 84 Earth Search";
    case "usgs.eros.m2m": return "USGS M2M";
    case "copernicus.dataspace.odata": return "Copernicus Data Space";
    default: return providerId;
  }
}

function sourceFallbackReasonLabel(reason: CoreWorkbenchSourceAttemptProjection["fallback_reason"]): string {
  switch (reason) {
    case "transient_network": return "网络异常";
    case "rate_limited": return "服务限流";
    case "server_error": return "服务端暂时错误";
    default: return "可恢复故障";
  }
}

function catalogFieldChipClass(label: string, value: string): string {
  if (label !== "Direction") return "border-success/25 bg-success/10 text-success";
  const normalized = value.trim().toUpperCase();
  if (normalized === "ASCENDING" || normalized === "ASC" || value === "升轨") {
    return "border-[#2563eb]/30 bg-[#2563eb]/10 text-[#2563eb] dark:text-[#93c5fd]";
  }
  if (normalized === "DESCENDING" || normalized === "DESC" || value === "降轨") {
    return "border-[#f97316]/30 bg-[#f97316]/10 text-[#ea580c] dark:text-[#fdba74]";
  }
  return "border-border bg-muted/40 text-muted-foreground";
}

export function CatalogWorkbenchDock({
  rows,
  products,
  selectedProductKeys,
  activeDownloadProductKeys,
  pausedDownloadProductKeys,
  taskProductKeys,
  demTasks,
  filterQuery,
  pageIndex,
  pageSize,
  boundedPageMode = false,
  totalCount = null,
  checkedCount = null,
  searchCounts = null,
  sourceAttempts = [],
  pageBusy = false,
  searchPending = false,
  searchPendingLabel = "正在检索并冻结新结果；上一批结果暂时隐藏。",
  revealProductKey = null,
  revealGeneration = 0,
  preferences,
  onPreferencesChange,
  onFilterQueryChange,
  onPageIndexChange,
  onActivateProduct,
  onToggleProduct,
  onShowProductInfo,
  onHideProductInfo,
  onCopyProductInfo,
  onPreviewProduct,
  assetDetailProductKeys,
  assetDetailsLoadingProductKey = null,
  onAssetDetailsProduct,
  onVisibleProductKeysChange,
  onPauseProduct,
  onResumeProduct,
  onDownloadProduct,
  onSelectAll,
  onClearSelection,
  onDownloadSelected,
  showDownloadSelected = true,
  rowDownloadLabel = "下载",
  acquisitionOptions = null,
  onPauseSelected,
  onResumeSelected,
  selectedPausableCount = 0,
  selectedResumableCount = 0,
  showSelectedTaskActions = false,
  taskPaused = false,
  resumeSelectedDisabledReason = null,
  rowDownloadDisabledReason = null,
  downloadBusy = false,
  downloadDisabledReason = null,
}: CatalogWorkbenchDockProps) {
  const dockRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);
  const lastRevealRef = useRef<string | null>(null);
  const resizeRef = useRef<{
    pointerId: number;
    startY: number;
    startHeight: number;
  } | null>(null);
  const [scrollTop, setScrollTop] = useState(
    () => scrollTopFromPageIndex(pageIndex, pageSize, CATALOG_ROW_HEIGHT),
  );
  const [viewportHeight, setViewportHeight] = useState(1);
  const rowWindow = virtualRowWindow(
    rows.length,
    scrollTop,
    viewportHeight,
    CATALOG_ROW_HEIGHT,
  );
  const visibleRows = rows.slice(rowWindow.start, rowWindow.end);
  const selectedCount = selectedProductKeys.size;
  const boundedTotalCount = typeof totalCount === "number" && Number.isSafeInteger(totalCount) && totalCount >= 0
    ? totalCount
    : rows.length;
  const boundedCheckedCount = typeof checkedCount === "number" && Number.isSafeInteger(checkedCount) && checkedCount >= 0
    ? checkedCount
    : selectedCount;
  const boundedPageCount = Math.max(1, Math.ceil(boundedTotalCount / Math.max(1, pageSize)));
  const boundedPageNumber = Math.min(boundedPageCount, Math.max(1, pageIndex + 1));
  const countSummary = boundedPageMode
    ? catalogSearchCountSummary({
        targetCount: searchCounts?.targetCount ?? null,
        providerMatchedCount: searchCounts?.providerMatchedCount ?? null,
        providerReturnedCount: searchCounts?.providerReturnedCount ?? null,
        frozenCandidateCount: searchCounts?.frozenCandidateCount ?? boundedTotalCount,
        currentPageCount: rows.length,
        checkedCount: boundedCheckedCount,
        truncated: searchCounts?.truncated ?? false,
      })
    : null;
  const hasCatalog = products.length > 0;
  const hasDem = demTasks.length > 0;
  const context = preferences.context === "dem" && hasDem
    ? "dem"
    : hasCatalog
      ? "catalog"
      : hasDem
        ? "dem"
        : "catalog";
  // Report viewport intent only; the owning workbench resolves and caches metadata.
  const visibleProductSignature = JSON.stringify(
    !searchPending && preferences.heightMode !== "collapsed" && context === "catalog"
      ? visibleRows.slice(0, 64).map(row => row.provider_product_key) : [],
  );
  useEffect(() => {
    onVisibleProductKeysChange?.(JSON.parse(visibleProductSignature) as string[]);
  }, [visibleProductSignature, onVisibleProductKeysChange]);
  useEffect(() => () => { onVisibleProductKeysChange?.([]); }, [onVisibleProductKeysChange]);
  const updatePreferences = (patch: Partial<DockPreferences>) => {
    onPreferencesChange({ ...preferences, ...patch, tab: "results" });
  };
  const availableDockHeight = () => {
    const measured = dockRef.current?.parentElement?.clientHeight;
    if (measured && Number.isFinite(measured)) return measured;
    return typeof window === "undefined" ? 620 : Math.max(620, window.innerHeight);
  };
  const availableHeight = availableDockHeight();
  const renderedHeight = preferences.heightMode === "collapsed"
    ? 44
    : clampDockHeight(preferences.heightPx, availableHeight);

  useLayoutEffect(() => {
    const element = listRef.current;
    if (!element) return;
    const update = () => setViewportHeight(Math.max(1, element.clientHeight));
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, [context, preferences.heightMode, preferences.heightPx]);

  useLayoutEffect(() => {
    const nextScrollTop = boundedPageMode
      ? 0
      : scrollTopFromPageIndex(pageIndex, pageSize, CATALOG_ROW_HEIGHT);
    setScrollTop(nextScrollTop);
    if (listRef.current && listRef.current.scrollTop !== nextScrollTop) {
      listRef.current.scrollTop = nextScrollTop;
    }
  }, [boundedPageMode, context, pageIndex, pageSize, preferences.heightMode, preferences.heightPx]);

  useLayoutEffect(() => {
    if (!revealProductKey || !listRef.current) return;
    const revealToken = `${revealGeneration}:${revealProductKey}`;
    if (lastRevealRef.current === revealToken) return;
    const nextScrollTop = scrollTopForProduct(rows, revealProductKey, CATALOG_ROW_HEIGHT);
    if (nextScrollTop === null) return;
    lastRevealRef.current = revealToken;
    listRef.current.scrollTop = nextScrollTop;
    setScrollTop(nextScrollTop);
    if (!boundedPageMode) {
      const nextPageIndex = pageIndexFromScrollTop(
        nextScrollTop,
        pageSize,
        CATALOG_ROW_HEIGHT,
      );
      if (nextPageIndex !== pageIndex) onPageIndexChange(nextPageIndex);
    }
  }, [boundedPageMode, pageIndex, pageSize, revealGeneration, revealProductKey, rows, onPageIndexChange]);

  const finishResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    resizeRef.current = null;
  };

  const handleResizeKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const delta = event.key === "ArrowUp" ? 24 : -24;
    updatePreferences({
      heightMode: "normal",
      heightPx: clampDockHeight(preferences.heightPx + delta, availableDockHeight()),
    });
  };

  return (
    <section
      ref={dockRef}
      data-catalog-workbench-dock
      aria-label="目录与 DEM 结果工作坞"
      className="relative z-[730] flex min-h-11 shrink-0 flex-col overflow-hidden border-t border-border bg-background/96 shadow-[0_-8px_28px_rgba(15,23,42,0.12)]"
      style={{ height: renderedHeight }}
    >
      {preferences.heightMode !== "collapsed" && (
        <div
          role="separator"
          aria-label="调整结果工作坞高度"
          aria-orientation="horizontal"
          aria-valuemin={180}
          aria-valuemax={Math.max(180, availableHeight - 160)}
          aria-valuenow={renderedHeight}
          tabIndex={0}
          className="absolute inset-x-0 top-0 z-20 h-2 cursor-ns-resize touch-none bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          onPointerDown={(event) => {
            event.preventDefault();
            event.currentTarget.setPointerCapture(event.pointerId);
            resizeRef.current = {
              pointerId: event.pointerId,
              startY: event.clientY,
              startHeight: renderedHeight,
            };
          }}
          onPointerMove={(event) => {
            const drag = resizeRef.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            updatePreferences({
              heightMode: "normal",
              heightPx: clampDockHeight(
                drag.startHeight + drag.startY - event.clientY,
                availableDockHeight(),
              ),
            });
          }}
          onPointerUp={finishResize}
          onPointerCancel={finishResize}
          onLostPointerCapture={() => {
            resizeRef.current = null;
          }}
          onKeyDown={handleResizeKeyDown}
        />
      )}
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-border/70 px-2.5">
        <Layers3 className="h-4 w-4 shrink-0 text-primary" />
        <span className="truncate text-xs font-semibold">
          {context === "catalog" ? "目录结果" : "DEM 任务结果"}
        </span>
        {hasCatalog && (
          boundedPageMode && countSummary ? (
            <div data-search-count-summary className="flex min-w-0 items-center gap-1 text-[11px]">
              {countSummary.target && (
                <Badge variant="neutral" data-search-count="target" data-testid="scope-target">{countSummary.target}</Badge>
              )}
              {countSummary.matched && (
                <Badge variant="neutral" data-search-count="matched" data-testid="scope-matched">{countSummary.matched}</Badge>
              )}
              <Badge variant="neutral" data-search-count="frozen" data-testid="scope-frozen">{countSummary.frozen}</Badge>
              <Badge variant="neutral" data-search-count="current" data-testid="scope-current">{countSummary.current}</Badge>
              <Badge variant="success" data-search-count="selected" data-testid="scope-selected">{countSummary.selected}</Badge>
            </div>
          ) : <Badge variant="neutral">{`${rows.length}/${products.length} 产品`}</Badge>
        )}
        {hasCatalog && sourceAttempts.length > 1 && (
          <details className="relative shrink-0 text-[11px]" data-source-attempts>
            <summary
              data-testid="source-attempts-summary"
              className="cursor-pointer list-none rounded-md border border-border bg-muted/35 px-2 py-1 text-muted-foreground hover:bg-muted"
              title="查看本次检索自动换源记录"
            >
              来源切换 {sourceAttempts.length - 1} 次
            </summary>
            <div className="absolute left-0 top-full z-40 mt-1 w-80 rounded-md border border-border bg-popover p-2 shadow-lg">
              <div className="mb-1 font-medium text-foreground">本次检索来源轨迹</div>
              <div className="space-y-1">
                {sourceAttempts.map((attempt, index) => (
                  <div key={`${attempt.provider_id}:${attempt.native_collection_id}:${index}`} className="rounded border border-border/60 px-2 py-1">
                    <div className="truncate font-medium text-foreground" title={`${attempt.provider_id} · ${attempt.native_collection_id}`}>
                      {sourceProviderLabel(attempt.provider_id)} · {attempt.native_collection_id}
                    </div>
                    <div className="text-muted-foreground">
                      {attempt.outcome === "succeeded"
                        ? "成功"
                        : `${sourceFallbackReasonLabel(attempt.fallback_reason)}，已自动切换`}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </details>
        )}
        {!boundedPageMode && selectedCount > 0 && <Badge variant="success">已选 {selectedCount}</Badge>}
        {hasDem && <Badge variant="neutral">DEM {demTasks.length}</Badge>}
        <div className="ml-auto flex shrink-0 items-center gap-1">
          {hasCatalog && hasDem && (
            <div className="mr-1 flex rounded-md border bg-muted/40 p-0.5 text-[11px]">
              <button
                type="button"
                className={cn("rounded px-2 py-1", context === "catalog" && "bg-background font-medium shadow-sm")}
                onClick={() => updatePreferences({ context: "catalog" })}
              >
                目录
              </button>
              <button
                type="button"
                className={cn("rounded px-2 py-1", context === "dem" && "bg-background font-medium shadow-sm")}
                onClick={() => updatePreferences({ context: "dem" })}
              >
                DEM
              </button>
            </div>
          )}
          <button
            type="button"
            aria-label={preferences.heightMode === "collapsed" ? "展开底部工作坞" : "折叠底部工作坞"}
            title={preferences.heightMode === "collapsed" ? "展开底部工作坞" : "折叠底部工作坞"}
            className="grid h-8 w-8 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground"
            onClick={() => updatePreferences({
              heightMode: preferences.heightMode === "collapsed" ? "normal" : "collapsed",
            })}
          >
            {preferences.heightMode === "collapsed"
              ? <ChevronUp className="h-4 w-4" />
              : <ChevronDown className="h-4 w-4" />}
          </button>
        </div>
      </header>

      {preferences.heightMode !== "collapsed" && context === "catalog" && (
        <div className="flex min-h-0 flex-1">
          <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex h-10 shrink-0 items-center gap-1.5 border-b border-border/60 px-2 text-xs">
            <Input
              value={filterQuery}
              onChange={(event) => onFilterQueryChange(event.currentTarget.value)}
              aria-label={boundedPageMode ? "筛选冻结结果" : "筛选目录产品"}
              placeholder={boundedPageMode ? "多关键词筛选：产品 / 平台 / 级别 / 时间 / 云量 / 轨道 / 瓦片" : "多关键词筛选产品 / 平台 / 级别 / 时间 / 云量 / 轨道 / 瓦片"}
              className="mr-auto h-7 min-w-[220px] max-w-[520px] flex-1 text-xs"
            />
            {filterQuery.trim() && (
              <Button size="sm" variant="ghost" className="h-7 px-2" onClick={() => onFilterQueryChange("")}>
                清空筛选
              </Button>
            )}
            <Button size="sm" variant="outline" className="h-7" onClick={onSelectAll} disabled={pageBusy}>
              全选
            </Button>
            <Button size="sm" variant="outline" className="h-7" onClick={onClearSelection} disabled={pageBusy}>
              重置
            </Button>
            {showSelectedTaskActions && (
              <>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7"
                  onClick={onPauseSelected}
                  disabled={selectedPausableCount === 0}
                  title={selectedPausableCount > 0 ? "暂停勾选且正在下载的产品" : "勾选正在下载的产品后可暂停"}
                >
                  <Pause className="h-3.5 w-3.5" />
                  暂停所选{selectedPausableCount > 0 ? ` (${selectedPausableCount})` : ""}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7"
                  onClick={onResumeSelected}
                  disabled={selectedResumableCount === 0 || Boolean(resumeSelectedDisabledReason)}
                  title={resumeSelectedDisabledReason || "继续勾选且已暂停的产品"}
                >
                  <Play className="h-3.5 w-3.5" />
                  继续所选{selectedResumableCount > 0 ? ` (${selectedResumableCount})` : ""}
                </Button>
              </>
            )}
            {acquisitionOptions}
            {showDownloadSelected && (
              <Button
                size="sm"
                className="h-7"
                onClick={onDownloadSelected}
                disabled={downloadBusy || (boundedPageMode ? boundedCheckedCount === 0 : selectedCount === 0) || Boolean(downloadDisabledReason)}
                title={downloadDisabledReason || "下载已勾选产品"}
              >
                {downloadBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Download className="h-3.5 w-3.5" />}
                下载所选
              </Button>
            )}
          </div>
          {boundedPageMode && (
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-border/60 px-2 py-1.5 text-[11px] text-muted-foreground">
              <span>
                筛选冻结结果 · 第 {boundedPageNumber}/{boundedPageCount} 页 · {countSummary?.current ?? `当前显示 ${rows.length}`} · {countSummary?.frozen ?? `已载入 ${boundedTotalCount}`} · {countSummary?.selected ?? `已选择 ${boundedCheckedCount}`}
              </span>
              <div className="flex items-center gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 px-2"
                  aria-label="上一页"
                  disabled={pageBusy || boundedPageNumber <= 1}
                  onClick={() => onPageIndexChange(boundedPageNumber - 2)}
                >
                  上一页
                </Button>
                <Input
                  type="number"
                  min={1}
                  max={boundedPageCount}
                  value={boundedPageNumber}
                  aria-label="结果页码"
                  className="h-7 w-16 px-2 text-center"
                  disabled={pageBusy}
                  onChange={(event) => {
                    const value = Number(event.currentTarget.value);
                    if (!Number.isSafeInteger(value)) return;
                    onPageIndexChange(Math.min(boundedPageCount, Math.max(1, value)) - 1);
                  }}
                />
                <span>/ {boundedPageCount}</span>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 px-2"
                  aria-label="下一页"
                  disabled={pageBusy || boundedPageNumber >= boundedPageCount}
                  onClick={() => onPageIndexChange(boundedPageNumber)}
                >
                  下一页
                </Button>
              </div>
            </div>
          )}
          {searchPending && (
            <div className="flex shrink-0 items-center gap-2 border-b border-primary/20 bg-primary/5 px-3 py-2 text-xs text-muted-foreground" data-catalog-search-pending>
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin motion-reduce:animate-none" />
              <span className="whitespace-normal break-words">{searchPendingLabel}</span>
            </div>
          )}
          <div
            ref={listRef}
            className="min-h-0 flex-1 overflow-auto"
            onScroll={(event) => {
              const nextScrollTop = event.currentTarget.scrollTop;
              setScrollTop(nextScrollTop);
              if (boundedPageMode) return;
              const nextPageIndex = pageIndexFromScrollTop(
                nextScrollTop,
                pageSize,
                CATALOG_ROW_HEIGHT,
              );
              if (nextPageIndex !== pageIndex) onPageIndexChange(nextPageIndex);
            }}
          >
            {!searchPending && rows.length === 0 ? (
              <div className="grid h-full min-h-20 place-items-center px-4 text-center text-xs text-muted-foreground">
                {products.length === 0
                  ? "未找到符合当前条件的数据。"
                  : "没有匹配当前筛选条件的产品；清空筛选可恢复完整结果。"}
              </div>
            ) : null}
            <div className="relative min-w-[820px]" style={{ height: rowWindow.totalHeight }}>
              {visibleRows.map((row, index) => {
                const top = (rowWindow.start + index) * CATALOG_ROW_HEIGHT;
                const downloading = activeDownloadProductKeys.has(row.provider_product_key);
                const paused = pausedDownloadProductKeys.has(row.provider_product_key);
                const taskMember = taskProductKeys.has(row.provider_product_key);
                return (
                  <div
                    key={row.provider_product_key}
                    role="button"
                    tabIndex={0}
                    aria-current={row.active ? "true" : undefined}
                    className={cn(
                      "absolute left-0 right-0 flex h-[72px] cursor-pointer items-center gap-2 border-b-2 border-border/70 bg-background px-2 py-1 text-xs hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                      row.active && "bg-primary/10 ring-1 ring-inset ring-primary/30",
                    )}
                    style={{ transform: `translateY(${top}px)` }}
                    onClick={() => onActivateProduct(row.provider_product_key)}
                    onKeyDown={(event) => {
                      if (event.target !== event.currentTarget) return;
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        onActivateProduct(row.provider_product_key);
                      }
                    }}
                  >
                    <input
                      type="checkbox"
                      checked={row.selected}
                      aria-label={`选择下载 ${row.display_name}`}
                      onClick={stopPropagation}
                      onKeyDown={stopPropagation}
                      onChange={(event) => onToggleProduct(row.provider_product_key, event.currentTarget.checked)}
                      className="h-4 w-4 shrink-0 accent-primary"
                    />
                    <div className="min-w-0 flex-1 self-stretch">
                      <div className="flex h-7 min-w-0 items-center gap-1.5">
                        <span className="min-w-0 flex-1 truncate font-mono text-[13px] font-semibold" title={row.display_name}>
                          {row.display_name}
                        </span>
                        <button
                          type="button"
                          aria-label={`定位并预览 ${row.display_name}`}
                          title={onPreviewProduct ? "在地图中定位并打开影像预览" : "影像预览将在影像与地图绑定后可用"}
                          disabled={!onPreviewProduct}
                          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          onClick={(event) => {
                            event.stopPropagation();
                            onPreviewProduct?.(row.provider_product_key);
                          }}
                          onKeyDown={stopPropagation}
                        >
                          <Eye className="h-3.5 w-3.5" />
                        </button>
                        <button
                          type="button"
                          aria-label={`查看并复制 ${row.display_name} 元数据`}
                          title="悬浮查看元数据，点击复制"
                          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                          onMouseEnter={(event) => onShowProductInfo(row.provider_product_key, event.currentTarget.getBoundingClientRect())}
                          onMouseLeave={onHideProductInfo}
                          onFocus={(event) => onShowProductInfo(row.provider_product_key, event.currentTarget.getBoundingClientRect())}
                          onBlur={onHideProductInfo}
                          onClick={(event) => {
                            event.stopPropagation();
                            onCopyProductInfo(row.provider_product_key);
                          }}
                          onKeyDown={stopPropagation}
                        >
                          <Info className="h-3.5 w-3.5" />
                        </button>
                        {assetDetailProductKeys?.has(row.provider_product_key) && onAssetDetailsProduct && (
                          <button
                            type="button"
                            aria-label={`查看 ${row.display_name} 波段与资产详情`}
                            title="查看完整文件集及远端总大小"
                            disabled={assetDetailsLoadingProductKey === row.provider_product_key}
                            className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground disabled:cursor-wait disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                            onClick={(event) => {
                              event.stopPropagation();
                              onAssetDetailsProduct(row.provider_product_key);
                            }}
                            onKeyDown={stopPropagation}
                          >
                            {assetDetailsLoadingProductKey === row.provider_product_key
                              ? <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                              : <Files className="h-3.5 w-3.5" />}
                          </button>
                        )}
                        {row.asset_summary && (
                          <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground" title={row.asset_summary}>
                            <Files className="h-3.5 w-3.5" />
                            {row.asset_summary}
                          </span>
                        )}
                      </div>
                      <div className="flex h-7 min-w-0 items-center gap-1 overflow-hidden text-[12px]">
                        <span className={cn("shrink-0 rounded-md border border-success/25 bg-success/10 px-1.5 py-0.5 font-medium text-success", row.kind === "optical" && "whitespace-nowrap text-center")} title={`平台: ${row.platform}`}>{row.platform}</span>
                        {row.download_restricted && (
                          <span className="flex shrink-0 items-center gap-1 rounded-md border border-warning/30 bg-warning/10 px-1.5 py-0.5 font-medium text-warning" title={restrictedSarReason(row.dataset)}>
                            <LockKeyhole className="h-3 w-3" />
                            受限
                          </span>
                        )}
                        <span className={cn("shrink-0 rounded-md border border-success/25 bg-success/10 px-1.5 py-0.5 font-medium text-success", row.kind === "optical" && "whitespace-nowrap text-center")} title={`产品/处理级别: ${row.processing_level}`}>{row.processing_level}</span>
                        <span className={cn("max-w-[190px] shrink truncate rounded-md border border-success/20 bg-success/10 px-1.5 py-0.5 font-mono font-medium text-success", row.kind === "optical" && "max-w-none shrink-0 whitespace-nowrap text-center")} title={row.acquisition_time}>{row.acquisition_time}</span>
                        {(row.file_size_label || row.kind === "optical") && (
                          <span className={cn("shrink-0 rounded-md border border-success/20 bg-success/10 px-1.5 py-0.5 font-mono font-medium text-success", row.kind === "optical" && "whitespace-nowrap text-center")} title={row.file_size_title ?? `远端文件大小: ${row.file_size_label ?? "未知"}`}>
                            {row.file_size_label ?? "—"}
                          </span>
                        )}
                        {row.domain_field_cells.map((cell) => (
                          <span key={cell.label} className={cn(
                            "shrink-0 rounded-md border px-1.5 py-0.5 font-medium",
                            catalogFieldChipClass(cell.label, cell.value),
                            row.kind === "optical" && "whitespace-nowrap text-center",
                          )} title={`${cell.label}: ${cell.value}`}>
                            {row.kind === "sar" && (cell.label === "Direction" || cell.label === "Polarization")
                              ? cell.value
                              : `${cell.label} ${cell.value}`}
                          </span>
                        ))}
                        {!row.footprint_available && (
                          <span className="flex shrink-0 items-center gap-1 rounded-md border px-1.5 py-0.5 text-muted-foreground">
                            <MapPinned className="h-3 w-3" />
                            无范围
                          </span>
                        )}
                      </div>
                    </div>
                    {rowAction(row, downloading, paused, taskMember, {
                      taskPaused,
                      downloadBusy,
                      rowDownloadDisabledReason,
                      onPauseProduct,
                      onResumeProduct,
                      onDownloadProduct,
                      rowDownloadLabel,
                    })}
                  </div>
                );
              })}
            </div>
          </div>
          </div>
        </div>
      )}

      {preferences.heightMode !== "collapsed" && context === "dem" && (
        <div className="min-h-0 flex-1 overflow-auto p-2">
          {demTasks.length === 0 ? (
            <div className="grid h-full min-h-20 place-items-center text-xs text-muted-foreground">
              暂无可展示的 DEM 任务结果。
            </div>
          ) : (
            <div className="grid gap-2 lg:grid-cols-2">
              {demTasks.map((task) => (
                <article key={task.task_id} className="rounded-md border bg-muted/20 p-2 text-xs">
                  <div className="flex items-center gap-2">
                    <Database className="h-3.5 w-3.5 shrink-0 text-primary" />
                    <span className="min-w-0 flex-1 truncate font-medium" title={task.name}>{task.name}</span>
                    <Badge variant={task.error ? "warning" : task.state === "finished" ? "success" : "neutral"}>{task.state}</Badge>
                  </div>
                  <div className="mt-1 flex min-w-0 items-center gap-2 font-mono text-[11px] text-muted-foreground">
                    <span className="min-w-0 flex-1 truncate" title={task.remote_source_summary}>{task.remote_source_summary}</span>
                    <span className="shrink-0">{task.bbox_summary}</span>
                  </div>
                  <div className="mt-2"><Progress value={task.progress.percent} indeterminate={!task.progress.determinate} /></div>
                  <div className="mt-1 flex gap-3 text-[11px] text-muted-foreground">
                    <span>{task.progress.percent == null ? "总量待获取" : `${task.progress.percent}%`}</span>
                    <span className="min-w-0 flex-1 truncate" title={task.current_stage}>{task.current_stage}</span>
                  </div>
                  {task.artifacts.length > 0 && (
                    <div className="mt-2 flex min-w-0 flex-wrap gap-1 border-t pt-1.5 text-[11px]">
                      {task.artifacts.map((artifact) => (
                        <span key={`${task.task_id}:${artifact.kind}`} className="max-w-full truncate rounded border px-1.5 py-0.5" title={artifact.path}>
                          {artifact.label}
                        </span>
                      ))}
                    </div>
                  )}
                  {task.error && <div className="mt-1 break-words text-destructive" title={task.error}>{task.error}</div>}
                </article>
              ))}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
