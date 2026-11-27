import React, { useMemo } from "react";
import { cn } from "../../lib/utils.ts";
import { useI18n } from "../../i18n/index.ts";
import type { ApplicationTask } from "../../shell/api/types.ts";
import {
  effectiveGroupStatus,
  groupOrbitCompanionTasks,
} from "../../shell/api/orbitChildRows.ts";
import {
  taskDataTypeKey,
  taskDataTypeLabel,
  taskMatchesStatusFilter,
  type DownloadStatusFilter,
} from "./downloadFilterLogic.ts";

export { taskDataTypeKey };
export type { DownloadStatusFilter };

export interface DownloadFilterBarProps {
  tasks: ApplicationTask[];
  selectedType: string;
  onSelectType: (type: string) => void;
  selectedStatus?: DownloadStatusFilter;
  onSelectStatus?: (status: DownloadStatusFilter) => void;
}

const STATUS_FILTER_OPTIONS: ReadonlyArray<{
  key: DownloadStatusFilter;
  labelKey:
    | "downloads.filter.statusAll"
    | "downloads.filter.statusActive"
    | "downloads.filter.statusPaused"
    | "downloads.filter.statusCompleted"
    | "downloads.filter.statusFailed";
}> = [
  { key: "all", labelKey: "downloads.filter.statusAll" },
  { key: "active", labelKey: "downloads.filter.statusActive" },
  { key: "paused", labelKey: "downloads.filter.statusPaused" },
  { key: "completed", labelKey: "downloads.filter.statusCompleted" },
  { key: "failed", labelKey: "downloads.filter.statusFailed" },
];

export function DownloadFilterBar({
  tasks,
  selectedType,
  onSelectType,
  selectedStatus = "all",
  onSelectStatus,
}: DownloadFilterBarProps) {
  const { t } = useI18n();

  const groupedTasks = useMemo(() => groupOrbitCompanionTasks(tasks), [tasks]);

  const tasksMatchingType = useMemo(
    () =>
      selectedType === "all"
        ? groupedTasks
        : groupedTasks.filter((group) => taskDataTypeKey(group.task) === selectedType),
    [groupedTasks, selectedType]
  );

  const tasksMatchingStatus = useMemo(
    () =>
      (selectedStatus === "all"
        ? groupedTasks
        : groupedTasks.filter((group) =>
            taskMatchesStatusFilter(effectiveGroupStatus(group), selectedStatus)
          )
      ).map((group) => group.task),
    [groupedTasks, selectedStatus]
  );

  const statusCounts = useMemo(() => {
    const counts: Record<DownloadStatusFilter, number> = {
      all: tasksMatchingType.length,
      active: 0,
      paused: 0,
      completed: 0,
      failed: 0,
    };
    for (const group of tasksMatchingType) {
      const status = effectiveGroupStatus(group);
      if (taskMatchesStatusFilter(status, "active")) counts.active += 1;
      else if (taskMatchesStatusFilter(status, "paused")) counts.paused += 1;
      else if (taskMatchesStatusFilter(status, "completed")) counts.completed += 1;
      else if (taskMatchesStatusFilter(status, "failed")) counts.failed += 1;
    }
    return counts;
  }, [tasksMatchingType]);

  const types = useMemo(() => {
    const seen = new Set<string>();
    for (const group of groupedTasks) {
      seen.add(taskDataTypeKey(group.task));
    }
    return Array.from(seen);
  }, [groupedTasks]);

  const typeCounts = useMemo(() => {
    const map = new Map<string, number>();
    for (const item of tasksMatchingStatus) {
      const key = taskDataTypeKey(item);
      map.set(key, (map.get(key) || 0) + 1);
    }
    return map;
  }, [tasksMatchingStatus]);

  return (
    <div className="flex flex-col gap-2">
      {onSelectStatus && (
        <div
          role="toolbar"
          data-testid="download-status-filter-bar"
          aria-label={t("downloads.filter.statusAria")}
          className="eo-segmented-track eo-segmented-track-wrap flex-wrap gap-1 rounded-[12px]"
        >
          {STATUS_FILTER_OPTIONS.map((opt) => {
            const count = statusCounts[opt.key];
            const isSelected = selectedStatus === opt.key;
            return (
              <button
                key={opt.key}
                type="button"
                data-testid={`download-status-filter-${opt.key}`}
                aria-pressed={isSelected}
                onClick={() => onSelectStatus(opt.key)}
                className={cn(
                  "eo-segmented-pill",
                  isSelected
                    ? "eo-segmented-pill-active"
                    : "text-sp-text-secondary hover:text-sp-text"
                )}
              >
                <span>{t(opt.labelKey)}</span>
                <span className="text-xs font-mono opacity-75">{count}</span>
              </button>
            );
          })}
        </div>
      )}

      <div
        role="toolbar"
        data-testid="download-dataset-filter-bar"
        aria-label={t("downloads.filter.aria")}
        className="eo-segmented-track eo-segmented-track-wrap flex-wrap gap-1 rounded-[12px]"
      >
        <button
          type="button"
          onClick={() => onSelectType("all")}
          className={cn(
            "eo-segmented-pill",
            selectedType === "all"
              ? "eo-segmented-pill-active"
              : "text-sp-text-secondary hover:text-sp-text"
          )}
        >
          <span>{t("downloads.filter.all")}</span>
          <span className="text-xs font-mono opacity-75">{tasksMatchingStatus.length}</span>
        </button>

        {types.map((typeKey) => {
          const count = typeCounts.get(typeKey) || 0;
          const isSelected = selectedType === typeKey;
          return (
            <button
              key={typeKey}
              type="button"
              onClick={() => onSelectType(typeKey)}
              className={cn(
                "eo-segmented-pill",
                isSelected
                  ? "eo-segmented-pill-active"
                  : "text-sp-text-secondary hover:text-sp-text"
              )}
            >
              <span>{taskDataTypeLabel(typeKey)}</span>
              <span className="text-xs font-mono opacity-75">{count}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
