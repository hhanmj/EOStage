import React, { useState, useEffect, useId, useMemo, useRef } from "react";
import {
  ChevronRight,
  HardDrive,
  X,
  Pause,
  Play,
  RotateCcw,
  RefreshCw,
  Info,
  FolderOpen,
  FileText,
  Scissors,
  ArrowUpToLine,
  Trash2,
} from "lucide-react";
import { useI18n, type MessageKey } from "../../i18n/index.ts";
import { cn } from "../../lib/utils.ts";
import type {
  ApplicationTask,
  TaskLifecycleStatus,
  TaskSceneItem,
  UnifiedTaskItemAction,
} from "../../shell/api/types.ts";
import { formatBytes } from "../../shell/api/rowProjection.ts";
import { useTaskEta } from "../../hooks/useLoadingPrimitives.ts";
import { isFixtureBackend } from "../../shell/api/fixture.ts";
import { openOutputDirectory } from "../../shell/api/index.ts";
import {
  formatTaskTimestamp,
  getAvailableTaskActions,
  isGacosWebformTask,
  isTaskFallbackEligible,
  isTaskHistoryRow,
  taskOutputDir,
} from "../../shell/api/taskLogic.ts";
import {
  companionStatusSummaryZh,
  type OrbitRowAction,
} from "../../shell/api/orbitChildRows.ts";
import { orbitItemAction } from "../../shell/api/orbitItemAction.ts";
import { pauseReasonLine, taskCardSizeLine } from "../../shell/api/taskCardLogic.ts";
import {
  lookupSceneItemSubsetEstimate,
  lookupSubsetSizeEstimate,
} from "../../lib/subsetSizeEstimateRegistry.ts";
import { explainErrorCode } from "../../shell/api/errorExplanations.ts";
import { resolveAccountActionFromError } from "../../shell/api/opticalCreateError.ts";
import { openTaskOutputErrorText, resolveTaskActionErrorText } from "../../shell/api/taskActionFlow.ts";
import { taskPageToSceneItems } from "../../shell/api/sceneProgress.ts";
import {
  executeExternalGis,
  loadApplicationTaskPage,
} from "../../lib/bridge.ts";
import { MAX_UI_TASK_PAGE_SIZE } from "../../lib/coreRestoreProjection.ts";
import {
  StatusBadge,
  type TaskCardProps,
  type TaskOutputRef,
} from "./DesktopDownloadDrawer.tsx";
import {
  normalizeTaskDisplayName,
  normalizeTaskProviderLabel,
  shouldHideRedundantDatasetToken,
  taskProductBadgeMeta,
} from "./downloadFilterLogic.ts";
import { OrbitChildRow, OrbitPageMessage } from "./OrbitChildRow.tsx";
import { TaskConflictCleanup } from "./TaskConflictCleanup.tsx";
import { RasterPixelProbePanel } from "./RasterPixelProbePanel.tsx";
import { exportDownloadFolderCatalogFiles } from "../../lib/downloadFolderCatalog.ts";
import { stripInternalDiagnosticSuffix } from "../../shell/api/resultListLogic.ts";

const SCENE_ITEM_STATUS_PRIORITY: Record<string, number> = {
  running: 0,
  transferring: 0,
  subsetting: 0,
  failed: 1,
  error: 1,
  paused: 2,
  queued: 3,
  pending: 3,
  completed: 4,
  succeeded: 4,
  cancelled: 5,
  interrupted: 5,
  skipped: 5,
};

/**
 * 纯函数：按活跃优先级稳定排序分景下载项（进行中 -> 失败 -> 已暂停 -> 排队中 -> 已完成 -> 已取消）
 */
export function sortTaskSceneItemsByPriority<T extends { status: string }>(
  items: readonly T[]
): T[] {
  return items
    .map((item, idx) => ({
      item,
      idx,
      rank: SCENE_ITEM_STATUS_PRIORITY[(item.status || "").trim().toLowerCase()] ?? 6,
    }))
    .sort((a, b) => (a.rank !== b.rank ? a.rank - b.rank : a.idx - b.idx))
    .map((entry) => entry.item);
}

interface OrbitTaskPage {
  taskId: string;
  revision: number;
  items: TaskSceneItem[];
  failed: boolean;
}

function useOrbitTaskPages(expanded: boolean, tasks: ApplicationTask[]) {
  const taskSignature = tasks.map((task) => `${task.task_id}:${task.revision}`).join("|");
  const [pages, setPages] = useState<Record<string, OrbitTaskPage>>({});
  const [retryVersions, setRetryVersions] = useState<Record<string, number>>({});
  const latestTasks = useRef(tasks);
  latestTasks.current = tasks;

  useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    setPages({});
    const requests = tasks.map(async (task) => {
      const failed = async () => {
        if (!cancelled) {
          setPages((current) => ({
            ...current,
            [task.task_id]: { taskId: task.task_id, revision: task.revision, items: [], failed: true },
          }));
        }
      };
      try {
        const result = await loadApplicationTaskPage({
          task_id: task.task_id,
          expected_revision: task.revision,
          page_index: 0,
          page_size: MAX_UI_TASK_PAGE_SIZE,
        });
        const current = latestTasks.current.find((entry) => entry.task_id === task.task_id);
        if (
          cancelled ||
          !result.ok ||
          result.page.task_id !== task.task_id ||
          result.page.revision !== task.revision ||
          current?.revision !== task.revision
        ) {
          if (!cancelled && result.ok && current?.revision === task.revision) await failed();
          return;
        }
        setPages((currentPages) => ({
          ...currentPages,
          [task.task_id]: {
            taskId: task.task_id,
            revision: task.revision,
            items: taskPageToSceneItems(result.page),
            failed: false,
          },
        }));
      } catch {
        await failed();
      }
    });
    void Promise.all(requests);
    return () => { cancelled = true; };
  }, [expanded, taskSignature, retryVersions]);

  const retry = (taskId: string) => {
    setPages((current) => {
      const next = { ...current };
      delete next[taskId];
      return next;
    });
    setRetryVersions((current) => ({ ...current, [taskId]: (current[taskId] ?? 0) + 1 }));
  };
  return { pages, retry };
}

function sceneStatusWord(status: TaskLifecycleStatus): MessageKey {
  switch (status) {
    case "completed":
      return "downloads.status.completed";
    case "running":
      return "downloads.status.downloading";
    case "paused":
      return "downloads.status.paused";
    case "failed":
      return "downloads.row.failed";
    case "queued":
      return "downloads.row.waiting";
    case "cancelled":
      return "downloads.status.cancelled";
    case "interrupted":
      return "downloads.status.interrupted";
    default:
      return "downloads.detail.unknown";
  }
}

function sceneStatusDotClass(status: TaskLifecycleStatus): string {
  switch (status) {
    case "completed":
      return "bg-sp-success-text";
    case "running":
      return "bg-sp-accent";
    case "paused":
      return "bg-sp-warning-text";
    case "failed":
      return "bg-sp-danger-text";
    default:
      return "bg-sp-text-disabled";
  }
}

function sceneProgressBarFill(status: TaskLifecycleStatus): string {
  switch (status) {
    case "completed":
      return "bg-sp-success-text";
    case "running":
      return "bg-sp-accent";
    case "paused":
      return "bg-sp-warning-text";
    case "failed":
      return "bg-sp-danger-text";
    default:
      return "bg-sp-line";
  }
}

export function DownloadTaskRow({
  task,
  compact = false,
  canRetryFallback,
  onPause,
  onResume,
  onRetry,
  onRetryFallback,
  onViewDetails,
  onOpenFolder = openOutputDirectory,
  onCancel,
  onDeleteHistory,
  onItemAction,
  priority = "normal",
  onTogglePriority,
  companions = [],
  independentOrbit = false,
  onOrbitAction = orbitItemAction,
  onOrbitAnnouncement,
  onConfigureAccount,
}: TaskCardProps) {
  const { t } = useI18n();
  const eta = useTaskEta(task);
  const panelId = useId();
  const [actionError, setActionError] = useState<string | null>(null);
  const [itemBusyKey, setItemBusyKey] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<boolean>(false);
  const [catalogExported, setCatalogExported] = useState<boolean>(false);
  const [externalGisDeliverState, setExternalGisDeliverState] = useState<
    "idle" | "busy" | "done" | "error"
  >("idle");
  const [externalGisDeliverMsg, setExternalGisDeliverMsg] = useState<string>("");
  const [scenePage, setScenePage] = useState<{
    taskId: string;
    revision: number;
    items: TaskSceneItem[];
    hiddenCount: number;
  } | null>(null);
  const latestTaskRef = useRef(task);
  latestTaskRef.current = task;
  const orbitTasks = independentOrbit ? [task] : companions;
  const { pages: orbitPages, retry: retryOrbitPage } = useOrbitTaskPages(expanded, orbitTasks);
  const [orbitAction, setOrbitAction] = useState<{ key: string; kind: OrbitRowAction } | null>(null);
  const [orbitActionErrors, setOrbitActionErrors] = useState<Record<string, string>>({});
  const activeTaskSnapshot = task.status === "running" || task.status === "queued" ? task : null;

  useEffect(() => {
    setActionError(null);
  }, [task.status, task.revision, task.updated_at]);

  const hasCachedSingleScene =
    scenePage?.taskId === task.task_id && scenePage.items.length > 0;
  const needSingleSceneAutoLoad =
    task.category === "scene_download" &&
    task.items_count === 1 &&
    !hasCachedSingleScene &&
    (!task.scene_items || task.scene_items.length === 0) &&
    (!task.scene_ids || task.scene_ids.length === 0);

  useEffect(() => {
    if ((!expanded && !needSingleSceneAutoLoad) || task.category !== "scene_download") return;
    let cancelled = false;
    const taskId = task.task_id;
    const revision = task.revision;

    const loadScenePage = async () => {
      try {
        const result = await loadApplicationTaskPage({
          task_id: taskId,
          expected_revision: revision,
          page_index: 0,
          page_size: MAX_UI_TASK_PAGE_SIZE,
        });
        const current = latestTaskRef.current;
        if (
          cancelled ||
          !result.ok ||
          result.page.task_id !== taskId ||
          current.task_id !== taskId ||
          (expanded && (result.page.revision !== revision || current.revision !== revision))
        ) return;
        const mappedItems = taskPageToSceneItems(result.page).map((mapped, index) => {
          const rawItemId = result.page.items[index]?.item?.item_id;
          return rawItemId && !mapped.item_id ? { ...mapped, item_id: rawItemId } : mapped;
        });
        setScenePage({
          taskId,
          revision,
          items: mappedItems,
          hiddenCount: Math.max(0, result.page.total_count - result.page.items.length),
        });
      } catch {
        // 展开前的场景清单继续作为静默回退。
      }
    };

    void loadScenePage();
    return () => {
      cancelled = true;
    };
  }, [expanded, needSingleSceneAutoLoad, task.task_id, task.revision, task.category, activeTaskSnapshot]);

  const currentScenePage = scenePage?.taskId === task.task_id && scenePage.revision === task.revision
    ? scenePage
    : null;
  const rawSceneItems = currentScenePage?.items ?? task.scene_items;
  const sceneItems = useMemo(
    () => (rawSceneItems ? sortTaskSceneItemsByPriority(rawSceneItems) : rawSceneItems),
    [rawSceneItems]
  );
  const taskEstimatedBytes = lookupSubsetSizeEstimate(task.task_id, task.scene_ids);
  const orbitTaskSignature = orbitTasks.map((entry) => `${entry.task_id}:${entry.revision}`).join("|");

  useEffect(() => {
    setOrbitAction(null);
    setOrbitActionErrors({});
  }, [orbitTaskSignature]);

  const runOrbitAction = async (companion: ApplicationTask, item: TaskSceneItem, kind: OrbitRowAction) => {
    if (orbitAction !== null || !item.item_id) return;
    const key = `orbit:${companion.task_id}:${item.item_id}`;
    setOrbitAction({ key, kind });
    setOrbitActionErrors((current) => ({ ...current, [key]: "" }));
    try {
      await onOrbitAction(companion, item, kind);
      const submittedKey =
        kind === "check"
          ? "downloads.row.orbitCheckSubmitted"
          : kind === "use_resorb"
            ? "downloads.row.orbitResorbSubmitted"
            : kind === "upgrade_poeorb"
              ? "downloads.row.orbitUpgradeSubmitted"
              : "downloads.row.orbitCancelSubmitted";
      onOrbitAnnouncement?.(t(submittedKey));
    } catch (error: unknown) {
      const failedKey =
        kind === "check"
          ? "downloads.row.orbitCheckFailed"
          : kind === "use_resorb"
            ? "downloads.row.orbitResorbFailed"
            : kind === "upgrade_poeorb"
              ? "downloads.row.orbitUpgradeFailed"
              : "downloads.row.orbitCancelFailed";
      setOrbitActionErrors((current) => ({
        ...current,
        [key]: resolveTaskActionErrorText(error, t(failedKey)),
      }));
    } finally {
      setOrbitAction(null);
    }
  };

  const orbitRowsForScene = (sceneId: string) => orbitTasks.flatMap((companion) => {
    const page = orbitPages[companion.task_id];
    if (!page || page.failed || page.taskId !== companion.task_id || page.revision !== companion.revision) return [];
    return page.items
      .filter((item) => item.scene_id === sceneId)
      .map((item) => {
        const key = `orbit:${companion.task_id}:${item.item_id}`;
        return (
          <OrbitChildRow
            key={key}
            task={companion}
            item={item}
            sourcePaused={task.status === "paused"}
            indented={!independentOrbit}
            revisionChanged={page.revision !== companion.revision}
            action={orbitAction?.key === key ? orbitAction.kind : null}
            error={orbitActionErrors[key] || null}
            onAction={(target, targetItem, kind) => void runOrbitAction(target, targetItem, kind)}
          />
        );
      });
  });

  const orbitPageMessages = () => orbitTasks.map((companion) => {
    const page = orbitPages[companion.task_id];
    if (!page || page.revision !== companion.revision) {
      return <OrbitPageMessage key={companion.task_id} failed={false} onRetry={() => retryOrbitPage(companion.task_id)} />;
    }
    if (page.failed) {
      return <OrbitPageMessage key={companion.task_id} failed onRetry={() => retryOrbitPage(companion.task_id)} />;
    }
    if (page.items.length === 0) {
      return <div role="listitem" key={companion.task_id} className="px-2 py-1 text-xs text-sp-text-muted">{t("downloads.row.emptyOrbitDetails")}</div>;
    }
    return null;
  });

  const runAction = async (fn?: () => void | Promise<void>) => {
    if (!fn) return;
    setActionError(null);
    try {
      await fn();
    } catch (err: unknown) {
      setActionError(resolveTaskActionErrorText(err, t("downloads.task.actionFailed")));
    }
  };

  const openFolder = async (open: (task: TaskOutputRef) => void | Promise<void>) => {
    setActionError(null);
    try {
      await open({ task_id: task.task_id, revision: task.revision });
    } catch (err: unknown) {
      setActionError(openTaskOutputErrorText(err));
    }
  };

  const runSceneItemAction = async (
    item: TaskSceneItem,
    action: UnifiedTaskItemAction,
  ) => {
    if (!onItemAction || !item.item_id || itemBusyKey) return;
    const key = `${item.item_id}:${action}`;
    setItemBusyKey(key);
    setActionError(null);
    try {
      await onItemAction(task, item, action);
    } catch (err: unknown) {
      setActionError(resolveTaskActionErrorText(err, t("downloads.task.actionFailed")));
    } finally {
      setItemBusyKey(null);
    }
  };

  const sceneCount =
    task.scene_items?.length || task.scene_ids?.length || task.items_count || 0;
  const hasMultiScene = sceneCount >= 1 || orbitTasks.length > 0;
  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const detailExpanded = !compact && (hasMultiScene ? expanded : detailsExpanded);
  const expandLabel = t(detailExpanded
    ? (orbitTasks.length > 0 ? "downloads.row.collapseScenesAndCompanions" : "downloads.row.collapseScenes")
    : (orbitTasks.length > 0 ? "downloads.row.expandScenesAndCompanions" : "downloads.row.expandScenes"));

  const isGacosWebform = isGacosWebformTask(task);
  const isHistoryRow = isTaskHistoryRow(task);
  const displayTitle = normalizeTaskDisplayName(task);
  const providerLabel = normalizeTaskProviderLabel(task.provider);
  const badgeMeta = taskProductBadgeMeta(task);
  const dataType =
    badgeMeta.label === "精密轨道 POEORB" && badgeMeta.label === displayTitle
      ? t("downloads.row.companionOrbitBadge")
      : isGacosWebform && badgeMeta.label === displayTitle
        ? t("downloads.row.gacosTypeBadge")
        : badgeMeta.label;
  const cachedSingleSceneId =
    scenePage?.taskId === task.task_id ? scenePage.items[0]?.scene_id : null;
  const singleSceneName =
    !isGacosWebform && sceneCount === 1
      ? sceneItems?.[0]?.scene_id || cachedSingleSceneId || task.scene_ids?.[0] || null
      : null;

  const hasTotalBytes =
    typeof task.total_bytes === "number" &&
    task.total_bytes > 0 &&
    task.bytes_reported !== false;
  const percent = hasTotalBytes
    ? Math.min(100, Math.round((task.done_bytes / task.total_bytes!) * 100))
    : null;

  const progressFillClass =
    task.status === "completed"
      ? "bg-sp-success-text"
      : task.status === "running"
        ? "bg-sp-accent"
        : task.status === "paused"
          ? "bg-sp-warning-text"
          : task.status === "failed"
            ? "bg-sp-danger-text"
            : "bg-sp-line";

  const outputDir = taskOutputDir(task);
  const replayableSearch =
    typeof canRetryFallback === "boolean"
      ? canRetryFallback
      : isTaskFallbackEligible(task);
  const rawActions = getAvailableTaskActions(
    task.status,
    outputDir !== null,
    replayableSearch,
  );
  const compactAllowed = new Set([
    "pause",
    "resume",
    "cancel",
    "retry",
    "retry_fallback",
    "details",
  ]);
  const scopedActions = compact
    ? rawActions.filter((action) => compactAllowed.has(action))
    : rawActions;
  const actions = isGacosWebform
    ? scopedActions.filter(
        (action) =>
          action !== "pause" &&
          action !== "resume" &&
          action !== "retry" &&
          action !== "retry_fallback" &&
          !(isHistoryRow && action === "cancel")
      )
    : scopedActions;

  // 拼接第二行摘要信息
  const line2Parts: string[] = [];
  if (providerLabel) line2Parts.push(providerLabel);
  if (task.dataset && !shouldHideRedundantDatasetToken(task.dataset)) {
    line2Parts.push(task.dataset);
  }
  if (isGacosWebform) {
    line2Parts.push(t("downloads.row.gacosSubmittedEmail"));
  } else {
    if (typeof task.items_count === "number" && task.items_count > 0) {
      const done =
        typeof task.completed_items_count === "number"
          ? task.completed_items_count
          : task.status === "completed"
            ? task.items_count
            : 0;
      line2Parts.push(
        task.items_unit === "files"
          ? t("downloads.row.fileCount", { done, total: task.items_count })
          : t("downloads.row.sceneCount", { done, total: task.items_count })
      );
    }
    const sizeText = taskCardSizeLine(task, formatBytes);
    if (sizeText) line2Parts.push(sizeText);
    if (task.status === "running" && task.speed_formatted) {
      line2Parts.push(task.speed_formatted);
    }
    if (task.status === "running" && eta.kind !== "hidden") {
      line2Parts.push(eta.textZh);
    }
  }

  const createdAtLabel = !compact ? formatTaskTimestamp(task.created_at) : null;
  const completedAtLabel =
    !compact && isHistoryRow
      ? formatTaskTimestamp(task.updated_at)
      : null;

  const pauseLine = pauseReasonLine(task.pause_reason);

  return (
    <div
      data-task-id={task.task_id}
      data-task-status={task.status}
      className={cn(
        "flex flex-col",
        task.status === "running"
          ? "eo-concentric-band"
          : "rounded-[10px] border border-sp-line bg-sp-card transition-colors hover:border-[#d4d4d8] dark:hover:border-[#3f3f46]"
      )}
    >
      <div
        className={cn(
          "flex min-h-12 items-center gap-2 px-3.5 py-2.5 transition-colors duration-fast ease-sp",
          task.status === "running" && "eo-concentric-inner"
        )}
      >
        {/* 左侧：完整下载中心显示展开按钮或 16px 占位；右上角轻量弹窗不展开分景细节 */}
        {!compact && (
          (hasMultiScene || !activeTaskSnapshot) ? (
            <button
              type="button"
              onClick={() => {
                if (hasMultiScene) {
                  if (expanded) setScenePage(null);
                  setExpanded((prev) => !prev);
                } else {
                  setDetailsExpanded((prev) => !prev);
                }
              }}
              aria-label={expandLabel}
              aria-expanded={detailExpanded}
              aria-controls={panelId}
              className="flex h-6 w-4 shrink-0 items-center justify-center text-sp-text-muted hover:text-sp-text cursor-pointer transition-colors"
            >
              <ChevronRight
                className={cn(
                  "h-4 w-4 transition-transform duration-fast",
                  expanded && hasMultiScene && "rotate-90",
                  detailsExpanded && !hasMultiScene && "rotate-90"
                )}
              />
            </button>
          ) : (
            <div className="w-4 shrink-0" />
          )
        )}

        {/* 主栏 */}
        <div className="min-w-0 flex-1 flex flex-col justify-center gap-1">
          {/* 行 1：数据名（长名截断不挤占右侧时间戳）+ 数据类型微标签（透明极简剪刀）+ 优先级 + 创建/结束时间 */}
          <div className="flex items-center justify-between gap-2 min-w-0">
            <div className="flex items-center gap-2 min-w-0 flex-1">
              <span
                className="min-w-0 shrink truncate text-sm font-semibold text-sp-text cursor-default"
                title={singleSceneName ? `${displayTitle} · ${singleSceneName}` : displayTitle}
              >
                {displayTitle}
              </span>
              <span
                className={cn(
                  "inline-flex items-center gap-1 shrink-0 rounded-sp-sm bg-sp-surface-muted px-2 py-1 text-xs text-sp-text-muted leading-tight",
                  badgeMeta.isSubset && "border border-sp-line text-sp-text-secondary"
                )}
                title={badgeMeta.isSubset ? t("downloads.row.subsetAoiDelivery", { label: dataType }) : dataType}
              >
                <span>{dataType}</span>
                {badgeMeta.isSubset && (
                  <span
                    data-testid="subset-scissors-badge"
                    className="inline-flex h-3.5 w-3.5 items-center justify-center bg-transparent text-sp-text-secondary"
                    title={t("downloads.row.subsetAoiShort")}
                  >
                    <Scissors className="-rotate-45 h-3 w-3" />
                  </span>
                )}
              </span>
              {priority === "high" && (
                <span
                  data-testid="task-priority-badge"
                  className="shrink-0 rounded-sp-sm bg-sp-warning-bg px-1.5 py-0.5 text-xs font-semibold text-sp-warning-text leading-tight"
                >
                  置顶
                </span>
              )}
            </div>
            {!compact && (createdAtLabel || completedAtLabel) && (
              <div
                data-testid="task-header-timestamps"
                className="shrink-0 whitespace-nowrap flex items-center gap-2 text-xs font-mono tabular-nums text-sp-text-muted"
              >
                {createdAtLabel && <span>创建时间 {createdAtLabel}</span>}
                {completedAtLabel && <span>结束时间 {completedAtLabel}</span>}
              </div>
            )}
          </div>

          {/* 进度条：跨越主栏宽度 */}
          <div className="w-full h-1 rounded-full bg-sp-line overflow-hidden">
            {task.status === "completed" ||
            (isGacosWebform && (task.status === "paused" || task.status === "queued")) ? (
              <div className="h-full w-full bg-sp-success-text rounded-full" />
            ) : hasTotalBytes && percent !== null ? (
              <div
                className={cn(
                  "h-full rounded-full transition-[width] duration-base ease-sp",
                  progressFillClass
                )}
                style={{ width: `${percent}%` }}
              />
            ) : task.status === "running" ? (
              <div className="h-full w-1/3 bg-sp-accent rounded-full progress-indeterminate-bar" />
            ) : null}
          </div>

          {/* 单景下载直接回显影像名称 */}
          {singleSceneName && (
            <div
              data-testid="single-scene-name"
              className="font-mono text-xs text-sp-text-secondary truncate"
              title={singleSceneName}
            >
              影像: {singleSceneName}
            </div>
          )}

          {/* 完整下载中心专属元信息：保存目录置顶于摘要行之前 */}
          {!compact && outputDir && (
            <div
              data-testid="task-page-metadata"
              className="text-xs font-mono text-sp-text-muted truncate"
              title={outputDir}
            >
              目录 {outputDir}
            </div>
          )}

          {/* 行 2：规范化 provider · 完成/总 景 · 已传/总量 字节 · 实时速度 */}
          <div
            data-testid="task-summary-line"
            className="text-xs text-sp-text-muted truncate"
          >
            {line2Parts.join(" · ")}
          </div>

          {!compact && !independentOrbit && companions.length > 0 && !expanded && (
            <div className="flex items-center">
              <button
                type="button"
                data-testid="task-companion-summary-pill"
                onClick={(e) => {
                  e.stopPropagation();
                  setExpanded(true);
                }}
                className="inline-flex items-center gap-1.5 rounded-full border border-sp-line bg-sp-surface-muted px-2.5 py-0.5 text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors cursor-pointer"
              >
                <span>
                  含 {companions.length} 项伴随精轨 · {companionStatusSummaryZh(companions)}
                </span>
              </button>
            </div>
          )}

          {/* 磁盘空间不足已暂停提示行 */}
          {pauseLine && (
            <div className="flex items-center gap-1 text-xs text-sp-warning-text">
              <HardDrive className="h-3 w-3 shrink-0" />
              <span>{pauseLine}</span>
            </div>
          )}

          {/* 操作报错提示行 */}
          {actionError && (
            <div role="alert" className="text-xs text-sp-danger-text">
              {actionError}
            </div>
          )}

          {/* 失败时的错误信息提示（兼容 task.error 与 Store 摘要级 task.failure） */}
          {task.status === "failed" && (() => {
            const directError = task.error
              ? typeof task.error === "string"
                ? task.error.trim()
                : task.error.message_zh?.trim() || ""
              : "";
            const firstFailureCode = task.failure?.codes?.[0] ?? null;
            const summaryError =
              task.failure?.message_zh?.trim()
              || (firstFailureCode ? explainErrorCode(firstFailureCode) || firstFailureCode : "")
              || (typeof task.failed_items_count === "number" && task.failed_items_count > 0
                ? `共 ${task.failed_items_count} 项下载失败，点击详情查看原因`
                : "");
            const visibleError = stripInternalDiagnosticSuffix(directError || summaryError) || (directError || summaryError);
            const accountAction = resolveAccountActionFromError(
              {
                code: (typeof task.error === "object" && task.error ? task.error.code : undefined) ?? task.failure?.codes?.[0],
                needs_account: typeof task.error === "object" && task.error ? task.error.needs_account : undefined,
                message_zh: visibleError,
              },
              task.dataset || task.provider || task.source_provider_id,
            );
            return visibleError ? (
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-sp-danger-text">
                <span className="min-w-0 truncate" title={visibleError}>
                  {visibleError}
                </span>
                {accountAction.actionAccountId && onConfigureAccount && (
                  <button
                    type="button"
                    data-testid="task-configure-account-btn"
                    onClick={(e) => {
                      e.stopPropagation();
                      onConfigureAccount(accountAction.actionAccountId ?? undefined);
                    }}
                    className="shrink-0 font-medium text-sp-accent underline hover:opacity-80 cursor-pointer"
                  >
                    {accountAction.actionLabel || t("accounts.loginCard.openSettings")}
                  </button>
                )}
              </div>
            ) : null;
          })()}
        </div>

        {/* 右侧：状态徽章与 28px 图标操作按钮 */}
        <div className="flex items-center gap-1 shrink-0 ml-2">
          {isGacosWebform &&
          (task.status === "paused" || task.status === "queued" || task.status === "completed") ? (
            <span
              data-testid="gacos-submitted-badge"
              className="inline-flex h-7 items-center gap-1 rounded-sp-pill bg-sp-success-bg px-2 text-xs font-medium text-sp-success-text"
            >
              已提交申请
            </span>
          ) : (
            <StatusBadge status={task.status} category={task.category} />
          )}

          {!compact && !isGacosWebform && onTogglePriority && (
              <button
                type="button"
                onClick={() => void runAction(onTogglePriority)}
                aria-pressed={priority === "high"}
                aria-label={priority === "high" ? "取消置顶" : "置顶任务"}
                title={
                  priority === "high"
                    ? "已置顶（点击取消置顶；在队列与历史列表中固定置顶显示）"
                    : "置顶任务（在列表中置顶显示；暂停中的任务同时恢复并优先调度）"
                }
                className={cn(
                  "sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                  priority === "high"
                    ? "bg-sp-accent/10 text-sp-accent-strong hover:bg-sp-hover"
                    : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
                )}
              >
                <ArrowUpToLine className="h-3.5 w-3.5" />
              </button>
            )}

          {actions.includes("cancel") && onCancel && (
            <button
              type="button"
              onClick={() => void runAction(onCancel)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.cancelTask")}
              title={t("downloads.task.cancelTask")}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("pause") && onPause && (
            <button
              type="button"
              onClick={() => void runAction(onPause)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.pauseTask")}
              title={t("downloads.task.pauseTask")}
            >
              <Pause className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("resume") && onResume && (
            <button
              type="button"
              onClick={() => void runAction(onResume)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-accent-strong hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.resumeTask")}
              title={t("downloads.task.resumeTask")}
            >
              <Play className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("retry") && onRetry && (
            <button
              type="button"
              onClick={() => void runAction(onRetry)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-warning-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.retryTask")}
              title={t("downloads.task.retryTask")}
            >
              <RotateCcw className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("retry_fallback") && onRetryFallback && (
            <button
              type="button"
              onClick={() => void runAction(onRetryFallback)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-warning-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.retryFallback")}
              title={t("downloads.task.retryFallback")}
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("details") && onViewDetails && (
            <button
              type="button"
              onClick={onViewDetails}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.viewDetails")}
              title={t("downloads.task.viewDetails")}
            >
              <Info className="h-3.5 w-3.5" />
            </button>
          )}

          {actions.includes("open_folder") && outputDir !== null && onOpenFolder && (
            <button
              type="button"
              onClick={() => void openFolder(onOpenFolder)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label={t("downloads.task.openFolderAria")}
              title={
                isFixtureBackend()
                  ? t("downloads.task.fixtureFolderHint")
                  : outputDir
              }
            >
              <FolderOpen className="h-3.5 w-3.5" />
            </button>
          )}

          {!compact && isHistoryRow && onDeleteHistory && (
            <button
              type="button"
              data-testid={`delete-history-task-${task.task_id}`}
              onClick={() => void runAction(onDeleteHistory)}
              className="sp-row-action-btn flex h-7 w-7 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-danger-bg hover:text-sp-danger-text transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
              aria-label="删除此记录"
              title="删除此记录（仅移出历史列表，不会删除已下载的本地文件）"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>

      {/* 展开的分景进度面板 */}
      {detailExpanded && (
        <div id={panelId} role="list" aria-label={t(orbitTasks.length > 0 ? "downloads.row.orbitList" : "downloads.row.sceneList")} className="mt-1 mb-2 ml-6 rounded-[10px] bg-sp-surface-muted/70 p-1.5 max-h-64 overflow-y-auto overflow-x-hidden space-y-0.5">
          <TaskConflictCleanup task={task} expanded={detailExpanded} />
          {!isGacosWebform && task.status === "completed" && (
            <div
              data-testid="download-catalog-bundle-bar"
              className="mb-1 flex flex-wrap items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-1.5 text-xs"
            >
              <span className="text-sp-text-secondary truncate">
                {externalGisDeliverState === "done" || externalGisDeliverState === "error"
                  ? externalGisDeliverMsg
                  : catalogExported
                    ? "已导出 catalog.geojson（可直接拖入 QGIS）与 README_数据说明.md"
                    : "导出 QGIS 目录清单文件 (catalog.geojson) 与数据说明 (README.md)"}
              </span>
              <div className="flex items-center gap-1.5 shrink-0">
                <button
                  type="button"
                  data-testid="export-qgis-catalog-btn"
                  onClick={() => {
                    exportDownloadFolderCatalogFiles(task, sceneItems ?? undefined);
                    setCatalogExported(true);
                  }}
                  className="inline-flex shrink-0 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0.5 text-xs font-semibold text-sp-accent hover:bg-sp-hover cursor-pointer"
                >
                  <FileText className="h-3 w-3" />
                  <span>{catalogExported ? "重新导出清单" : "导出清单文件"}</span>
                </button>
                {(() => {
                  const deliverDir = taskOutputDir(task);
                  if (!deliverDir) return null;
                  return (
                    <button
                      type="button"
                      data-testid="deliver-external-gis-btn"
                      disabled={externalGisDeliverState === "busy"}
                      onClick={() => {
                        setExternalGisDeliverState("busy");
                        void executeExternalGis({
                          toolchain: "qgis",
                          action: "deliver_project",
                          input_paths: [deliverDir],
                          output_path: deliverDir,
                          open_gui: false,
                        })
                          .then((res) => {
                            if (res && res.ok) {
                              setExternalGisDeliverState("done");
                              setExternalGisDeliverMsg(
                                res.project_path ||
                                  t("shell.pluginsPage.deliverToGisDone"),
                              );
                            } else {
                              setExternalGisDeliverState("error");
                              setExternalGisDeliverMsg(
                                t("shell.pluginsPage.deliverToGisFailed"),
                              );
                            }
                          })
                          .catch((err: unknown) => {
                            setExternalGisDeliverState("error");
                            setExternalGisDeliverMsg(
                              err instanceof Error
                                ? err.message
                                : t("shell.pluginsPage.deliverToGisFailed"),
                            );
                          });
                      }}
                      className="inline-flex shrink-0 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0.5 text-xs font-semibold text-sp-text hover:bg-sp-hover cursor-pointer disabled:opacity-50"
                    >
                      <FolderOpen className="h-3 w-3" />
                      <span>
                        {externalGisDeliverState === "done"
                          ? t("shell.pluginsPage.deliverToGisDone")
                          : t("shell.pluginsPage.deliverToGisBtn")}
                      </span>
                    </button>
                  );
                })()}
              </div>
            </div>
          )}
          {!isGacosWebform &&
            task.status === "completed" &&
            outputDir !== null &&
            !String(task.category).includes("orbit") && (() => {
              const taskBbox = (task as { bbox?: [number, number, number, number] }).bbox;
              const defaultLon =
                Array.isArray(taskBbox) &&
                taskBbox.length === 4 &&
                Number.isFinite(taskBbox[0]) &&
                Number.isFinite(taskBbox[2])
                  ? (taskBbox[0] + taskBbox[2]) * 0.5
                  : null;
              const defaultLat =
                Array.isArray(taskBbox) &&
                taskBbox.length === 4 &&
                Number.isFinite(taskBbox[1]) &&
                Number.isFinite(taskBbox[3])
                  ? (taskBbox[1] + taskBbox[3]) * 0.5
                  : null;
              return (
                <RasterPixelProbePanel
                  sourcePath={outputDir}
                  defaultLon={defaultLon}
                  defaultLat={defaultLat}
                />
              );
            })()}
          {isGacosWebform ? (
            <div
              data-testid="gacos-submitted-summary"
              className="rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-2 text-xs text-sp-text-secondary leading-relaxed"
            >
              已向 www.gacos.net 提交大气延迟申请，处理完成后结果将直接发送至您配置的接收邮箱。
            </div>
          ) : sceneItems && sceneItems.length > 0 ? (
            <>
              {sceneItems.map((item) => {
                const itemTotal =
                  typeof item.total_bytes === "number" && item.total_bytes > 0
                    ? item.total_bytes
                    : null;
                const fallbackItemEst =
                  itemTotal === null && item.status !== "completed"
                    ? (lookupSceneItemSubsetEstimate(task.task_id, item.scene_id) ??
                      (taskEstimatedBytes && sceneItems.length > 0
                        ? Math.round(taskEstimatedBytes / sceneItems.length)
                        : null))
                    : null;
                const itemDenominator = itemTotal ?? fallbackItemEst;
                const itemPercent = itemDenominator
                  ? Math.min(itemTotal ? 100 : 99, Math.round((item.done_bytes / itemDenominator) * 100))
                  : item.status === "completed"
                    ? 100
                    : 0;

                return (
                  <React.Fragment key={`scene:${item.item_id ?? item.scene_id}`}>
                    <div
                      role="listitem"
                      className="sp-row-compact flex flex-col gap-0.5 rounded-sp-sm px-2 py-1 text-xs transition-colors duration-fast ease-sp hover:bg-sp-hover"
                    >
                      <div className="flex items-center gap-2.5 min-w-0">
                        <span
                          className={cn(
                            "h-1.5 w-1.5 rounded-full shrink-0",
                            sceneStatusDotClass(item.status)
                          )}
                        />
                        <span
                          className="font-mono text-xs truncate flex-1 text-sp-text"
                          title={item.scene_id}
                        >
                          {item.scene_id}
                        </span>
                        <div className="w-24 h-1 rounded-full bg-sp-line overflow-hidden shrink-0">
                          <div
                            className={cn(
                              "h-full rounded-full transition-[width] duration-base",
                              sceneProgressBarFill(item.status)
                            )}
                            style={{ width: `${itemPercent}%` }}
                          />
                        </div>
                        <span className="font-mono tabular-nums text-xs text-sp-text-muted shrink-0 text-right min-w-[7.75rem] whitespace-nowrap">
                          {itemTotal ? (
                            <>
                              {formatBytes(item.done_bytes)} / {formatBytes(itemTotal)}
                            </>
                          ) : item.status === "completed" ? (
                            formatBytes(item.done_bytes)
                          ) : fallbackItemEst ? (
                            <>
                              {item.done_bytes > 0 ? `${formatBytes(item.done_bytes)} / ` : ""}
                              {t("shared.taskCard.estimatedTotal", { total: formatBytes(fallbackItemEst) })}
                            </>
                          ) : (
                            <>
                              {formatBytes(item.done_bytes)} / --
                            </>
                          )}
                        </span>
                        <span className="text-xs text-sp-text-muted shrink-0 w-14 text-right">
                          {t(sceneStatusWord(item.status))}
                        </span>
                        {!compact &&
                          !isGacosWebform &&
                          !independentOrbit &&
                          Boolean(item.item_id) &&
                          Boolean(onItemAction) && (
                            <div className="flex items-center gap-0.5 shrink-0">
                              {(item.status === "running" || item.status === "queued") && (
                                <button
                                  type="button"
                                  disabled={itemBusyKey !== null}
                                  onClick={() => void runSceneItemAction(item, "pause")}
                                  aria-label={`暂停 ${item.scene_id}`}
                                  title="暂停此景下载"
                                  className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-surface hover:text-sp-text disabled:opacity-50 cursor-pointer"
                                >
                                  <Pause className="h-3 w-3" />
                                </button>
                              )}
                              {item.status === "paused" && (
                                <button
                                  type="button"
                                  disabled={itemBusyKey !== null}
                                  onClick={() => void runSceneItemAction(item, "resume")}
                                  aria-label={`继续 ${item.scene_id}`}
                                  title="继续此景下载"
                                  className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-accent-strong hover:bg-sp-surface disabled:opacity-50 cursor-pointer"
                                >
                                  <Play className="h-3 w-3" />
                                </button>
                              )}
                              {(item.status === "failed" ||
                                item.status === "cancelled" ||
                                item.status === "interrupted") && (
                                <button
                                  type="button"
                                  disabled={itemBusyKey !== null}
                                  onClick={() => void runSceneItemAction(item, "retry")}
                                  aria-label={`重试 ${item.scene_id}`}
                                  title="重试此景下载"
                                  className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-warning-text hover:bg-sp-surface disabled:opacity-50 cursor-pointer"
                                >
                                  <RotateCcw className="h-3 w-3" />
                                </button>
                              )}
                              {(item.status === "running" ||
                                item.status === "queued" ||
                                item.status === "paused") && (
                                <button
                                  type="button"
                                  disabled={itemBusyKey !== null}
                                  onClick={() => void runSceneItemAction(item, "cancel")}
                                  aria-label={`取消 ${item.scene_id}`}
                                  title="取消此景下载"
                                  className="flex h-6 w-6 items-center justify-center rounded-sp-sm text-sp-text-secondary hover:bg-sp-danger-bg hover:text-sp-danger-text disabled:opacity-50 cursor-pointer"
                                >
                                  <X className="h-3 w-3" />
                                </button>
                              )}
                            </div>
                          )}
                      </div>
                      {item.failure?.reason_zh && (() => {
                        const cleanReason =
                          stripInternalDiagnosticSuffix(item.failure.reason_zh) ||
                          item.failure.reason_zh;
                        return (
                          <div
                            className="pl-4 text-xs text-sp-danger-text truncate"
                            title={cleanReason}
                          >
                            {cleanReason}
                          </div>
                        );
                      })()}
                    </div>
                    {orbitRowsForScene(item.scene_id)}
                  </React.Fragment>
                );
              })}
              {orbitPageMessages()}
              {currentScenePage && currentScenePage.hiddenCount > 0 && (
                <div className="pt-1 px-2 text-xs text-sp-text-muted">
                  {t("downloads.row.hiddenScenes", { amount: currentScenePage.hiddenCount })}
                </div>
              )}
            </>
          ) : task.scene_ids && task.scene_ids.length > 0 ? (
            <>
              <div className="text-xs text-sp-text-muted font-semibold pb-1 px-2">
                {t("downloads.row.sceneSummary", {
                  completed: task.completed_items_count ?? 0,
                  failed: task.failed_items_count ?? 0,
                  total: task.items_count || task.scene_ids.length,
                })}
              </div>
              {task.scene_ids.map((id) => (
                <div key={id} className="flex h-7 items-center gap-2.5 rounded-sp-sm px-2 text-xs hover:bg-sp-hover transition-colors duration-fast ease-sp">
                  <span className="h-1.5 w-1.5 rounded-full bg-sp-text-disabled shrink-0" />
                  <span
                    className="font-mono text-xs truncate flex-1 text-sp-text"
                    title={id}
                  >
                    {id}
                  </span>
                </div>
              ))}
              {orbitPageMessages()}
            </>
          ) : detailExpanded ? (
            <div className="py-2 px-2 text-xs text-sp-text-muted">{t("downloads.row.emptyScenes")}</div>
          ) : orbitTasks.length > 0 ? (
            orbitPageMessages()
          ) : (
            <div className="py-2 px-2 text-xs text-sp-text-muted">{t("downloads.row.emptyScenes")}</div>
          )}
        </div>
      )}
    </div>
  );
}
