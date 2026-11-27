/**
 * AgentMultiBatchResultsView.tsx
 * 会话级多数据源结果并存切换条 + 地图/结果清单/下载任务同屏联动视图：
 * - 支持同一会话/同一轮检索出的多个数据源（如 Sentinel-1 + Sentinel-2 + NISAR + Landsat）全部保留；
 * - 默认支持「全部」合并查看，也可一键按单个数据源胶囊筛选并联动地图视野；
 * - 统一接入排序（数据源 / 日期 / 大小）、左图/右图卷帘对比、行内下载任务状态与「在地图工作台打开」。
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  CheckSquare,
  ChevronDown,
  ChevronRight,
  Download,
  ExternalLink,
  FolderOpen,
  Layers,
  Maximize2,
  Minimize2,
  Pause,
  Play,
  Square,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { SceneRow } from "@/lib/bridge";
import type { ApplicationTask, RowAction, RowTaskState } from "@/shell/api/types";
import {
  canonicalAgentDatasetShortName,
  type AgentSessionBatchEntry,
} from "@/shell/api/agentSessionWorkspace";
import { groupScenesByStack, sceneStackGroupKey } from "@/shell/api/resultGrouping";
import {
  DEFAULT_SORT_STATE,
  nextSort,
  sortKeysFor,
  sortScenes,
  type SortState,
} from "@/shell/api/resultListLogic";
import { extractCloudCover } from "@/shell/api/rowProjection";
import { CatalogResultRow } from "../CatalogResultRow";
import { inferResourceIdFromSceneId } from "./AgentMapSceneDrawer";
import { EO_MONOCHROME_ICON_CLASS } from "./agentToolSemantics";

export function resolveAgentBatchShortName(
  batch: Pick<AgentSessionBatchEntry, "batchId" | "resourceId" | "scenes">,
  sourceNames?: Readonly<Record<string, string>>,
  batchLabelsById?: Readonly<Record<string, string>>
): string {
  return canonicalAgentDatasetShortName(
    batch.resourceId,
    batch.scenes,
    sourceNames,
    batchLabelsById?.[batch.batchId] ?? null
  );
}

export interface AgentMultiBatchPillBarProps {
  batches: readonly AgentSessionBatchEntry[];
  activeFilter: "all" | string;
  combinedCount: number;
  sourceNames?: Readonly<Record<string, string>>;
  batchLabelsById?: Readonly<Record<string, string>>;
  onSelectFilter: (filter: "all" | string) => void;
  compact?: boolean;
  className?: string;
}

export function AgentMultiBatchPillBar({
  batches,
  activeFilter,
  combinedCount,
  sourceNames,
  batchLabelsById,
  onSelectFilter,
  compact = false,
  className,
}: AgentMultiBatchPillBarProps) {
  const { t } = useI18n();
  if (batches.length <= 1) return null;

  const isAllActive = activeFilter === "all" || !batches.some((b) => b.batchId === activeFilter);

  return (
    <div
      data-testid="agent-multi-batch-bar"
      role="tablist"
      aria-label="多数据源结果切换"
      className={cn(
        "flex shrink-0 items-center gap-1 overflow-x-auto no-scrollbar select-none",
        compact ? "py-0.5" : "px-3 py-1.5 border-b border-sp-divider bg-sp-surface",
        className
      )}
    >
      <button
        type="button"
        role="tab"
        data-testid="agent-batch-pill-all"
        aria-selected={isAllActive}
        onClick={() => onSelectFilter("all")}
        className={cn(
          "shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors cursor-pointer tabular-nums",
          isAllActive
            ? "bg-sp-text text-sp-surface shadow-xs"
            : "bg-sp-surface-muted text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text border border-sp-line"
        )}
      >
        {t("agent.multiBatch.allDatasets", { count: combinedCount })}
      </button>

      {batches.map((b) => {
        const isSelected = !isAllActive && activeFilter === b.batchId;
        const count = Math.max(b.totalCount, b.scenes.length);
        const name = resolveAgentBatchShortName(b, sourceNames, batchLabelsById);
        return (
          <button
            key={b.batchId}
            type="button"
            role="tab"
            data-testid={`agent-batch-pill-${b.batchId}`}
            aria-selected={isSelected}
            onClick={() => onSelectFilter(b.batchId)}
            className={cn(
              "shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors cursor-pointer tabular-nums",
              isSelected
                ? "bg-sp-text text-sp-surface shadow-xs"
                : "bg-sp-surface-muted text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text border border-sp-line"
            )}
          >
            {t("agent.multiBatch.datasetCount", { name, count })}
          </button>
        );
      })}
    </div>
  );
}

type AgentSortKey = "date" | "source" | "size";

export interface AgentUnifiedResultsDeckProps {
  scenes: SceneRow[];
  totalCount: number | null;
  batches: readonly AgentSessionBatchEntry[];
  activeFilter: "all" | string;
  combinedCount: number;
  sourceNames?: Readonly<Record<string, string>>;
  batchLabelsById?: Readonly<Record<string, string>>;
  onSelectFilter: (filter: "all" | string) => void;
  defaultBatchId?: string | null;
  resolveBatchIdForScene?: (scene: SceneRow) => string | null | undefined;
  rowStates?: Map<string, RowTaskState>;
  tasksById?: Map<string, ApplicationTask>;
  sessionTasks?: ApplicationTask[];
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "cancel" | "retry"
  ) => Promise<void>;
  onOpenDownloadCenter?: (taskId?: string) => void;
  selectedSceneId?: string | null;
  locateRequest?: { sceneId: string; seq: number } | null;
  onSelectScene?: (sceneId: string | null) => void;
  onHoverSceneChange?: (sceneId: string | null) => void;
  checkedSceneIds: Set<string>;
  onToggleCheckScene?: (sceneId: string) => void;
  onToggleCheckAll?: () => void;
  onRowAction?: (itemId: string, action: RowAction) => void;
  onPreviewScene?: (scene: SceneRow) => void;
  isPreviewPinnedFor?: (sceneId: string) => boolean;
  onTogglePinPreview?: (scene: SceneRow) => void;
  creatingItemIds: Set<string>;
  onDownloadChecked?: () => void;
  downloadBusy?: boolean;
  onOpenInMapWorkbench?: () => void;
  isFullscreenDock?: boolean;
  onToggleFullscreenDock?: () => void;
  onCollapseSplit?: () => void;
  className?: string;
}

export function AgentUnifiedResultsDeck({
  scenes,
  totalCount,
  batches,
  activeFilter,
  combinedCount,
  sourceNames,
  batchLabelsById,
  onSelectFilter,
  defaultBatchId,
  resolveBatchIdForScene,
  rowStates,
  tasksById,
  sessionTasks = [],
  onTaskAction,
  onOpenDownloadCenter,
  selectedSceneId,
  locateRequest,
  onSelectScene,
  onHoverSceneChange,
  checkedSceneIds,
  onToggleCheckScene,
  onToggleCheckAll,
  onRowAction,
  onPreviewScene,
  isPreviewPinnedFor,
  onTogglePinPreview,
  creatingItemIds,
  onDownloadChecked,
  downloadBusy = false,
  onOpenInMapWorkbench,
  isFullscreenDock = false,
  onToggleFullscreenDock,
  onCollapseSplit,
  className,
}: AgentUnifiedResultsDeckProps) {
  const { t } = useI18n();
  const [sortState, setSortState] = useState<SortState>(DEFAULT_SORT_STATE);
  const [stackGrouped, setStackGrouped] = useState<boolean>(false);
  const [collapsedStackKeys, setCollapsedStackKeys] = useState<Set<string>>(new Set());
  const [expandedFailureIds, setExpandedFailureIds] = useState<Set<string>>(new Set());
  const [highlightedLocateId, setHighlightedLocateId] = useState<string | null>(null);
  const resultsScrollRef = useRef<HTMLDivElement | null>(null);

  const toggleStackCollapse = (key: string) => {
    setCollapsedStackKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const hasOpticalCloud = useMemo(
    () => scenes.some((sc) => extractCloudCover(sc) !== null),
    [scenes]
  );
  const hasSarOrbitOrFrame = useMemo(
    () => scenes.some((sc) => typeof sc.relative_orbit === "number" || typeof sc.frame === "number"),
    [scenes]
  );
  const availableSortKeys = useMemo(
    () =>
      sortKeysFor("mixed", hasOpticalCloud, {
        hasOpticalKeys: hasOpticalCloud,
        hasSarKeys: hasSarOrbitOrFrame,
        hasSourceKey: batches.length > 1,
      }),
    [batches.length, hasOpticalCloud, hasSarOrbitOrFrame]
  );
  const effectiveSort = useMemo<SortState>(() => {
    if (availableSortKeys.some((k) => k.key === sortState.key)) {
      return sortState;
    }
    return { key: "date", dir: sortState.dir };
  }, [availableSortKeys, sortState]);

  const toggleFailureExpand = (id: string) => {
    setExpandedFailureIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const sortedScenes = useMemo(
    () =>
      sortScenes(
        scenes,
        effectiveSort.key,
        effectiveSort.dir,
        (sc) => inferResourceIdFromSceneId(sc.scene_id) || sc.platform || sc.provider || ""
      ),
    [effectiveSort.dir, effectiveSort.key, scenes]
  );

  const sortedGroups = useMemo(() => {
    if (!stackGrouped) return [];
    return groupScenesByStack(sortedScenes, { sortByStackSize: true });
  }, [sortedScenes, stackGrouped]);

  useEffect(() => {
    if (!locateRequest?.sceneId) return;
    const targetId = locateRequest.sceneId;
    const inCurrent = scenes.find((s) => s.scene_id === targetId);
    if (!inCurrent && activeFilter !== "all") {
      onSelectFilter("all");
      return;
    }
    if (inCurrent && stackGrouped) {
      const targetGroupKey = sceneStackGroupKey(inCurrent);
      setCollapsedStackKeys((prev) => {
        if (!prev.has(targetGroupKey)) return prev;
        const next = new Set(prev);
        next.delete(targetGroupKey);
        return next;
      });
    }
    setHighlightedLocateId(targetId);
    const clearHighlight = window.setTimeout(() => {
      setHighlightedLocateId((prev) => (prev === targetId ? null : prev));
    }, 1800);
    const scrollTimer = window.setTimeout(() => {
      const container = resultsScrollRef.current;
      if (!container) return;
      const escaped = window.CSS?.escape
        ? window.CSS.escape(targetId)
        : targetId.replaceAll('"', '\\"');
      const el = container.querySelector(`[data-scene-id="${escaped}"]`);
      if (el && typeof (el as HTMLElement).scrollIntoView === "function") {
        const prefersReducedMotion =
          typeof window.matchMedia === "function" &&
          window.matchMedia("(prefers-reduced-motion: reduce)").matches;
        (el as HTMLElement).scrollIntoView({
          block: "nearest",
          behavior: prefersReducedMotion ? "auto" : "smooth",
        });
      }
    }, 50);
    return () => {
      window.clearTimeout(clearHighlight);
      window.clearTimeout(scrollTimer);
    };
  }, [locateRequest?.seq, locateRequest?.sceneId, scenes, activeFilter, onSelectFilter, stackGrouped]);

  const activeOrRecentTasks = useMemo(
    () =>
      sessionTasks
        .filter((tk) =>
          ["running", "queued", "paused", "completed"].includes(tk.status)
        )
        .slice(0, 2),
    [sessionTasks]
  );

  return (
    <div
      data-testid="agent-unified-results-deck"
      className={cn(
        "flex flex-col flex-1 min-h-0 w-full overflow-hidden bg-sp-card",
        className
      )}
    >
      {/* 1. 多数据源并存切换胶囊栏（>=2 类数据时呈现「全部 + 各数据源」） */}
      <AgentMultiBatchPillBar
        batches={batches}
        activeFilter={activeFilter}
        combinedCount={combinedCount}
        sourceNames={sourceNames}
        batchLabelsById={batchLabelsById}
        onSelectFilter={onSelectFilter}
      />

      {/* 2. 全选 + 排序（日期 / 数据源 / 大小）+ 批量下载 + 在地图工作台打开 */}
      <div className="flex flex-wrap items-center justify-between gap-2 px-3 py-1.5 border-b border-sp-divider text-xs text-sp-text-secondary bg-sp-surface-muted shrink-0">
        <div className="flex items-center gap-2 min-w-0">
          <button
            type="button"
            onClick={onToggleCheckAll}
            aria-label={t("agent.workbench.selectAll")}
            className="flex items-center text-sp-text-secondary hover:text-sp-text cursor-pointer transition-colors"
          >
            {scenes.length > 0 && checkedSceneIds.size >= scenes.length ? (
              <CheckSquare className="h-4 w-4 text-sp-text" />
            ) : (
              <Square className="h-4 w-4 text-sp-text-muted" />
            )}
          </button>
          <span className="truncate tabular-nums">
            {t("agent.workbench.resultsCount", { count: scenes.length })}
            {totalCount && totalCount > scenes.length ? ` / ${totalCount}` : ""}
            {checkedSceneIds.size > 0
              ? t("agent.workbench.selectedCount", { count: checkedSceneIds.size })
              : ""}
          </span>

          {/* 同幅归组开关 + 统一排序胶囊 */}
          <button
            type="button"
            data-testid="btn-agent-dock-stack-toggle"
            aria-pressed={stackGrouped}
            onClick={() => setStackGrouped((prev) => !prev)}
            title={t("mapWorkbench.results.groupByFrameTitle")}
            className={cn(
              "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs transition-colors cursor-pointer select-none border",
              stackGrouped
                ? "bg-sp-text text-sp-surface border-sp-text font-medium"
                : "border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
            )}
          >
            <Layers className="h-3 w-3 shrink-0" />
            <span>{t("mapWorkbench.results.groupByFrame")}</span>
          </button>

          <div className="hidden sm:inline-flex items-center gap-0.5 pl-1 border-l border-sp-line">
            {availableSortKeys.map((item) => {
              const active = effectiveSort.key === item.key;
              return (
                <button
                  key={item.key}
                  type="button"
                  onClick={() => setSortState((prev) => nextSort(prev, item.key))}
                  className={cn(
                    "inline-flex items-center gap-0.5 rounded px-1.5 py-0.5 text-xs transition-colors cursor-pointer select-none whitespace-nowrap",
                    active
                      ? "bg-sp-hover font-medium text-sp-text"
                      : "text-sp-text-muted hover:text-sp-text"
                  )}
                >
                  <span>{item.label}</span>
                  {active &&
                    (effectiveSort.dir === "asc" ? (
                      <ArrowUp className="h-3 w-3" />
                    ) : (
                      <ArrowDown className="h-3 w-3" />
                    ))}
                </button>
              );
            })}
          </div>
        </div>

        <div className="flex items-center gap-1.5 shrink-0">
          {onDownloadChecked && checkedSceneIds.size > 0 && (
            <button
              type="button"
              data-testid="btn-agent-batch-download"
              onClick={onDownloadChecked}
              disabled={downloadBusy}
              className="inline-flex items-center gap-1 px-2.5 py-1 text-xs font-medium rounded-sp-sm bg-sp-accent-fill text-white hover:opacity-90 disabled:opacity-50 cursor-pointer transition-colors"
            >
              <Download className="h-3 w-3" />
              <span>{t("agent.workbench.batchDownload")}</span>
            </button>
          )}

          {onOpenInMapWorkbench && (
            <button
              type="button"
              data-testid="btn-open-in-map-workbench"
              onClick={onOpenInMapWorkbench}
              title="同步至手动检索台"
              aria-label="同步至手动检索台"
              className="inline-flex h-6 w-6 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
            >
              <ExternalLink className={cn("h-3.5 w-3.5 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
            </button>
          )}

          {onToggleFullscreenDock && (
            <button
              type="button"
              data-testid="btn-toggle-dock-fullscreen"
              onClick={onToggleFullscreenDock}
              title={isFullscreenDock ? "还原分屏高度" : "全屏展开数据坞"}
              aria-label={isFullscreenDock ? "还原分屏高度" : "全屏展开数据坞"}
              className="inline-flex h-6 w-6 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
            >
              {isFullscreenDock ? (
                <Minimize2 className="h-3.5 w-3.5" />
              ) : (
                <Maximize2 className="h-3.5 w-3.5" />
              )}
            </button>
          )}

          {onCollapseSplit && (
            <button
              type="button"
              data-testid="btn-collapse-map-split-dock"
              onClick={onCollapseSplit}
              title={t("agent.multiBatch.collapseSplitList")}
              aria-label={t("agent.multiBatch.collapseSplitList")}
              className="inline-flex h-6 w-6 items-center justify-center rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text cursor-pointer transition-colors"
            >
              <ChevronDown className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>

      {/* 3. 本会话关联下载任务内联状态条（消除切去「任务与成果」才能看进度的割裂感） */}
      {activeOrRecentTasks.length > 0 && (
        <div
          data-testid="agent-inline-tasks-strip"
          className="flex flex-wrap items-center justify-between gap-2 border-b border-sp-divider bg-sp-surface px-3 py-1.5 text-xs shrink-0"
        >
          <div className="flex flex-wrap items-center gap-2 min-w-0">
            {activeOrRecentTasks.map((tk) => (
              <div
                key={tk.task_id}
                className="inline-flex items-center gap-1.5 rounded-full border border-sp-line bg-sp-surface-muted px-2.5 py-0.5 text-xs"
              >
                <Layers className={cn("h-3 w-3 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
                <span className="max-w-[160px] truncate font-medium text-sp-text">
                  {tk.name || tk.dataset || tk.task_id}
                </span>
                <span className="text-sp-text-muted tabular-nums">
                  {tk.status === "completed"
                    ? "已完成"
                    : tk.status === "paused"
                      ? "已暂停"
                      : "下载中"}
                </span>
                {onTaskAction && tk.status === "running" && (
                  <button
                    type="button"
                    title="暂停"
                    onClick={() => void onTaskAction(tk.task_id, tk.revision, "pause")}
                    className="text-sp-text-secondary hover:text-sp-text cursor-pointer"
                  >
                    <Pause className="h-3 w-3" />
                  </button>
                )}
                {onTaskAction && tk.status === "paused" && (
                  <button
                    type="button"
                    title="继续"
                    onClick={() => void onTaskAction(tk.task_id, tk.revision, "resume")}
                    className="text-sp-text-secondary hover:text-sp-text cursor-pointer"
                  >
                    <Play className="h-3 w-3" />
                  </button>
                )}
                {onOpenDownloadCenter && (
                  <button
                    type="button"
                    title="查看任务详情"
                    onClick={() => onOpenDownloadCenter(tk.task_id)}
                    className="text-sp-text-secondary hover:text-sp-text cursor-pointer"
                  >
                    <FolderOpen className="h-3 w-3" />
                  </button>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 4. 逐景科学结果列表（支持平铺与可折叠同幅归组、整组全选、快视图预览、i 信息卡与行内下载状态） */}
      <div
        ref={resultsScrollRef}
        data-testid="agent-results-scroll-container"
        className="flex-1 min-h-0 overflow-y-auto p-2 flex flex-col gap-1.5"
      >
        {(() => {
          const renderSceneRow = (scene: SceneRow) => {
            const rState = rowStates?.get(scene.scene_id);
            const linkedTask = rState?.task_id ? tasksById?.get(rState.task_id) : undefined;
            const sceneBatchId = resolveBatchIdForScene?.(scene) ?? defaultBatchId ?? undefined;
            const isHighlighted = highlightedLocateId === scene.scene_id;
            return (
              <CatalogResultRow
                key={scene.scene_id}
                scene={scene}
                className={isHighlighted ? "bg-sp-selected/60" : undefined}
                batchId={sceneBatchId}
                rowState={rState}
                task={linkedTask}
                isSelected={selectedSceneId === scene.scene_id}
                isChecked={checkedSceneIds.has(scene.scene_id)}
                isCreating={creatingItemIds.has(scene.scene_id)}
                isFailureExpanded={expandedFailureIds.has(scene.scene_id)}
                onToggleFailureExpand={toggleFailureExpand}
                onSelectScene={(id: string) => onSelectScene?.(id)}
                onHoverChange={onHoverSceneChange}
                onToggleCheckScene={(id: string) => onToggleCheckScene?.(id)}
                onRowAction={(id: string, act: RowAction) => onRowAction?.(id, act)}
                isPreviewPinned={isPreviewPinnedFor?.(scene.scene_id) ?? false}
                onTogglePinPreview={onTogglePinPreview ?? onPreviewScene}
              />
            );
          };

          if (!stackGrouped) {
            return sortedScenes.map(renderSceneRow);
          }

          return sortedGroups.map((group) => {
            const groupCollapsed = collapsedStackKeys.has(group.key);
            const groupIds = group.items.map((sc) => sc.scene_id);
            const allGroupChecked =
              groupIds.length > 0 && groupIds.every((id) => checkedSceneIds.has(id));
            return (
              <div
                key={group.key}
                data-testid="agent-dock-stack-group"
                className="shrink-0 rounded-sp-sm border border-sp-line bg-sp-surface/60 overflow-hidden"
              >
                <div className="flex items-center justify-between gap-2 px-2.5 py-1.5 text-xs text-sp-text bg-sp-surface-muted border-b border-sp-line/60">
                  <button
                    type="button"
                    onClick={() => toggleStackCollapse(group.key)}
                    aria-expanded={!groupCollapsed}
                    className="flex items-center gap-1.5 min-w-0 flex-1 text-left font-medium cursor-pointer select-none hover:opacity-85"
                  >
                    {groupCollapsed ? (
                      <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                    ) : (
                      <ChevronDown className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" />
                    )}
                    <span className="truncate">{group.label}</span>
                    <span className="shrink-0 rounded-full bg-sp-hover px-2 py-0.5 text-xs font-medium text-sp-text-secondary tabular-nums">
                      {t("mapWorkbench.results.frameGroupCount", { count: group.items.length })}
                    </span>
                  </button>
                  {onToggleCheckScene && (
                    <button
                      type="button"
                      onClick={(e) => {
                        e.stopPropagation();
                        for (const id of groupIds) {
                          const checked = checkedSceneIds.has(id);
                          if (allGroupChecked ? checked : !checked) {
                            onToggleCheckScene(id);
                          }
                        }
                      }}
                      className="shrink-0 rounded-sp-sm border border-sp-line bg-sp-surface px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                    >
                      {allGroupChecked
                        ? t("mapWorkbench.results.unselectGroup")
                        : t("mapWorkbench.results.selectGroup")}
                    </button>
                  )}
                </div>
                {!groupCollapsed && (
                  <div className="p-1.5 flex flex-col gap-1">
                    {group.items.map(renderSceneRow)}
                  </div>
                )}
              </div>
            );
          });
        })()}
      </div>
    </div>
  );
}
