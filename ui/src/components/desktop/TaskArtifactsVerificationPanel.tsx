import React, { useMemo } from "react";
import { CheckCircle2, FolderOpen, Loader2, Pause, AlertCircle } from "lucide-react";
import { useI18n } from "@/i18n";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";
import { cn } from "@/lib/utils";
import type { ApplicationTask } from "@/shell/api/types";
import { taskOutputDir } from "@/shell/api/taskLogic";
import { formatBytes } from "@/shell/api/rowProjection";
import {
  effectiveGroupStatus,
  groupOrbitCompanionTasks,
  companionStatusSummaryZh,
} from "@/shell/api/orbitChildRows";
import {
  normalizeTaskDisplayName,
  normalizeTaskProviderLabel,
  taskDataTypeKey,
  taskDataTypeLabel,
  taskMatchesStatusFilter,
} from "./downloadFilterLogic.ts";

export interface TaskOutputFolderRef {
  task_id: string;
  revision: number;
}

export interface TaskArtifactsVerificationPanelProps {
  tasks: ApplicationTask[];
  onOpenFolder?: (task: TaskOutputFolderRef) => void | Promise<void>;
  onOpenTask?: (taskId: string) => void;
  compact?: boolean;
}

export function TaskArtifactsVerificationPanel({
  tasks,
  onOpenFolder,
  onOpenTask,
  compact = false,
}: TaskArtifactsVerificationPanelProps) {
  const { t } = useI18n();
  const groups = useMemo(() => groupOrbitCompanionTasks(tasks), [tasks]);
  const activeCount = useMemo(
    () =>
      groups.filter((g) => taskMatchesStatusFilter(effectiveGroupStatus(g), "active")).length,
    [groups]
  );

  if (groups.length === 0) {
    return (
      <div className={cn("flex flex-1 items-center justify-center", compact ? "p-3" : "")}>
        <EmptyState
          kind="first-use"
          size="card"
          testId="artifacts-verification-panel"
          visual={<EmptyStateIcon name="folder" size={22} />}
          title={t("downloads.artifacts.emptyTitle")}
          className="w-full"
        />
      </div>
    );
  }

  return (
    <div
      data-testid="artifacts-verification-panel"
      className={cn("flex flex-col gap-3", compact ? "p-3 overflow-y-auto flex-1 min-h-0" : "")}
    >
      {activeCount > 0 && (
        <div
          data-testid="artifacts-independent-transfer-banner"
          className="flex items-center gap-2 rounded-sp-md border border-sp-accent/30 bg-sp-selected/60 px-3 py-2 text-xs text-sp-text"
        >
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sp-accent" aria-hidden="true" />
          <span>
            {t("downloads.artifacts.activeIndependentBanner", { count: activeCount })}
          </span>
        </div>
      )}

      <div className="space-y-2.5">
        {groups.map((group) => {
          const { task, companions } = group;
          const groupStatus = effectiveGroupStatus(group);
          const outputDir = taskOutputDir(task);
          const title = normalizeTaskDisplayName(task);
          const providerLabel = normalizeTaskProviderLabel(task.provider);
          const dataType = taskDataTypeLabel(taskDataTypeKey(task));
          const totalBytes = typeof task.total_bytes === "number" && task.total_bytes > 0 ? task.total_bytes : null;
          const pct =
            task.status === "completed"
              ? 100
              : totalBytes
                ? Math.min(99, Math.round((task.done_bytes / totalBytes) * 100))
                : typeof task.items_count === "number" &&
                    task.items_count > 0 &&
                    typeof task.completed_items_count === "number"
                  ? Math.min(99, Math.round((task.completed_items_count / task.items_count) * 100))
                  : 0;

          const countText =
            typeof task.items_count === "number" && task.items_count > 0
              ? `${
                  typeof task.completed_items_count === "number"
                    ? task.completed_items_count
                    : task.status === "completed"
                      ? task.items_count
                      : 0
                }/${task.items_count} ${
                  task.items_unit === "files"
                    ? t("downloads.artifacts.unitFiles")
                    : t("downloads.artifacts.unitScenes")
                }`
              : null;

          return (
            <div
              key={task.task_id}
              data-testid={`artifact-verification-card-${task.task_id}`}
              className="rounded-card border border-sp-line bg-sp-card p-3 space-y-2"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-2">
                  <span className="truncate text-sm font-semibold text-sp-text" title={title}>
                    {title}
                  </span>
                  <span className="shrink-0 rounded-sp-sm bg-sp-surface-muted px-2 py-0.5 text-xs text-sp-text-secondary">
                    {dataType}
                  </span>
                  {companions.length > 0 && (
                    <span className="shrink-0 rounded-full border border-sp-line bg-sp-surface px-2 py-0.5 text-xs text-sp-text-muted">
                      {t("downloads.artifacts.companionsSummary", {
                        count: companions.length,
                        status: companionStatusSummaryZh(companions),
                      })}
                    </span>
                  )}
                </div>

                {groupStatus === "completed" ? (
                  <span
                    data-testid="artifact-status-verified"
                    className="inline-flex shrink-0 items-center gap-1 rounded-full bg-sp-success-bg px-2.5 py-0.5 text-xs font-medium text-sp-success-text"
                  >
                    <CheckCircle2 className="h-3.5 w-3.5" aria-hidden="true" />
                    <span>{t("downloads.artifacts.statusVerified")}</span>
                  </span>
                ) : groupStatus === "running" || groupStatus === "queued" ? (
                  <span
                    data-testid="artifact-status-running"
                    className="inline-flex shrink-0 items-center gap-1 rounded-full bg-sp-selected px-2.5 py-0.5 text-xs font-medium text-sp-accent-strong tabular-nums"
                  >
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                    <span>{t("downloads.artifacts.statusRunningPct", { pct })}</span>
                  </span>
                ) : groupStatus === "paused" ? (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-sp-warning-bg px-2.5 py-0.5 text-xs font-medium text-sp-warning-text">
                    <Pause className="h-3.5 w-3.5" aria-hidden="true" />
                    <span>{t("downloads.artifacts.statusPaused")}</span>
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-sp-danger-bg px-2.5 py-0.5 text-xs font-medium text-sp-danger-text">
                    <AlertCircle className="h-3.5 w-3.5" aria-hidden="true" />
                    <span>{t("downloads.artifacts.statusBlocked")}</span>
                  </span>
                )}
              </div>

              <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-sp-text-muted tabular-nums">
                {providerLabel && <span>{providerLabel}</span>}
                {countText && <span>· {countText}</span>}
                <span>
                  · {t("downloads.artifacts.savedBytesPrefix")} {formatBytes(task.done_bytes)}
                  {totalBytes ? ` / ${formatBytes(totalBytes)}` : ""}
                </span>
              </div>

              <div className="flex flex-wrap items-center justify-between gap-2 rounded-sp-sm bg-sp-surface-muted px-2.5 py-1.5 text-xs">
                <span
                  className="min-w-0 flex-1 truncate font-mono text-sp-text-secondary"
                  title={outputDir || t("downloads.artifacts.defaultOutputDir")}
                >
                  {outputDir || t("downloads.artifacts.defaultOutputDir")}
                </span>
                <div className="flex items-center gap-1.5 shrink-0">
                  {outputDir && onOpenFolder && (
                    <button
                      type="button"
                      data-testid="artifact-open-folder-btn"
                      onClick={() => void onOpenFolder(task)}
                      className="inline-flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-card px-2 py-1 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                    >
                      <FolderOpen className="h-3.5 w-3.5" aria-hidden="true" />
                      <span>{t("downloads.artifacts.openFolder")}</span>
                    </button>
                  )}
                  {onOpenTask && (
                    <button
                      type="button"
                      data-testid="artifact-open-task-btn"
                      onClick={() => onOpenTask(task.task_id)}
                      className="inline-flex items-center rounded-sp-sm border border-sp-line bg-sp-card px-2 py-1 text-xs font-medium text-sp-accent-strong hover:bg-sp-hover transition-colors cursor-pointer"
                    >
                      {t("downloads.artifacts.viewTask")}
                    </button>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
