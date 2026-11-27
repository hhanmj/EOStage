import React, { useEffect, useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateIcon } from "@/components/ui/EmptyStateVisuals";
import { cn } from "../../lib/utils.ts";
import { useI18n } from "../../i18n/index.ts";
import type { AgentWorkItem } from "../../lib/generated/v3-contracts.ts";
import type { ApplicationTask } from "../../shell/api/types.ts";
import { runStatusLabel } from "../../shell/api/agentStreamLogic.ts";
import { formatRunStarted } from "../../shell/api/agentRunBadges.ts";
import { runUsageText } from "../../shell/api/agentUsage.ts";
import {
  type AgentWorkFilter,
  filterWork,
  formatDuration,
  isRunActive,
  joinRunTasks,
  runDurationMs,
  sortWorkNewestFirst,
  taskProgressPercent,
  taskStatusLabel,
  workCounts,
  workTitle,
} from "../../shell/api/agentWork.ts";

export interface AgentWorkPanelProps {
  /** null：没有会话或没有桥接（显示空态） */
  work: AgentWorkItem[] | null;
  tasks: ApplicationTask[];
  onOpenTask?: (taskId: string) => void;
}

const FILTERS: { id: AgentWorkFilter; label: string }[] = [
  { id: "all", label: "全部" },
  { id: "active", label: "进行中" },
  { id: "ended", label: "已结束" },
];

function runStatusClass(status: string): string {
  switch (status) {
    case "running": return "bg-sp-surface-muted text-sp-accent-strong";
    case "completed": return "bg-sp-success-bg text-sp-success-text";
    case "failed": return "bg-sp-danger-bg text-sp-danger-text";
    default: return "bg-sp-warning-bg text-sp-warning-text";
  }
}

function taskDotClass(task: ApplicationTask | null): string {
  switch (task?.status) {
    case "completed": return "bg-sp-success-text";
    case "running": return "bg-sp-accent";
    case "paused": return "bg-sp-warning-text";
    case "failed": return "bg-sp-danger-text";
    default: return "bg-sp-text-disabled";
  }
}

function WorkEmptyState({ label }: { label: string }) {
  return (
    <div role="status" className="flex flex-1 items-center justify-center p-4">
      <EmptyState
        kind="first-use"
        size="card"
        visual={<EmptyStateIcon name="folder" size={22} />}
        title={label}
        className="w-full max-w-xs"
      />
    </div>
  );
}

export function AgentWorkPanel({ work, tasks, onOpenTask }: AgentWorkPanelProps) {
  const { t } = useI18n();
  const [filter, setFilter] = useState<AgentWorkFilter>("all");
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  const sorted = useMemo(() => sortWorkNewestFirst(work ?? []), [work]);
  const counts = useMemo(() => workCounts(sorted), [sorted]);
  const visible = useMemo(() => filterWork(sorted, filter), [sorted, filter]);
  const selected = visible.find((item) => item.run.run_id === selectedRunId) ?? visible[0] ?? null;
  const rows = useMemo(() => (selected ? joinRunTasks(selected, tasks) : []), [selected, tasks]);

  // 进行中运行的时长随父组件的任务轮询刷新，不另起计时器
  useEffect(() => setNowMs(Date.now()), [work, tasks]);

  if (work === null || sorted.length === 0) {
    return <WorkEmptyState label="暂无运行" />;
  }

  const usage = selected ? runUsageText(selected.run.usage_total) : null;

  return (
    <div className="flex flex-1 min-h-0 flex-col overflow-hidden">
      <div className="flex shrink-0 flex-col gap-2 px-3 pt-3 pb-2">
        <div role="radiogroup" aria-label="运行筛选" className="flex items-center justify-between gap-2">
          <div className="flex items-center gap-0.5 rounded-sp-md bg-sp-hover p-0.5">
            {FILTERS.map((f) => (
              <button
                key={f.id}
                type="button"
                role="radio"
                aria-checked={filter === f.id}
                onClick={() => setFilter(f.id)}
                className={cn(
                  "flex h-7 items-center rounded-sp-sm px-2.5 text-xs outline-none transition-colors duration-fast ease-sp cursor-pointer focus-visible:ring-1 focus-visible:ring-sp-accent",
                  filter === f.id
                    ? "bg-sp-surface font-medium text-sp-text shadow-card dark:bg-sp-card"
                    : "text-sp-text-secondary hover:text-sp-text"
                )}
              >
                {f.label} {counts[f.id]}
              </button>
            ))}
          </div>
        </div>

        {/* EOStage Segmented Progress Bar (分段式运行状态条) */}
        <div
          data-testid="eo-segmented-progress"
          aria-hidden="true"
          className="flex h-1.5 w-full items-center gap-1 overflow-hidden rounded-full"
        >
          {sorted.map((item) => (
            <span
              key={item.run.run_id}
              className={cn(
                "h-full flex-1 rounded-full transition-colors duration-fast",
                item.run.status === "completed"
                  ? "bg-sp-status-success"
                  : item.run.status === "running"
                  ? "bg-sp-accent"
                  : item.run.status === "failed"
                  ? "bg-sp-status-danger"
                  : "bg-sp-line"
              )}
            />
          ))}
        </div>
      </div>

      {visible.length === 0 ? (
        <WorkEmptyState label="暂无运行" />
      ) : (
        <>
          <ul aria-label="运行列表" className="flex max-h-[45%] shrink-0 flex-col gap-1 overflow-y-auto px-2 pb-2">
            {visible.map((item) => {
              const isSelected = selected?.run.run_id === item.run.run_id;
              const duration = formatDuration(runDurationMs(item, nowMs));
              const title = workTitle(item);
              return (
                <li key={item.run.run_id}>
                  <button
                    type="button"
                    aria-pressed={isSelected}
                    onClick={() => setSelectedRunId(item.run.run_id)}
                    className={cn(
                      "flex w-full flex-col gap-1 rounded-sp-md px-3 py-2 text-left transition-colors duration-fast ease-sp cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                      isSelected ? "bg-sp-selected text-sp-text font-medium" : "hover:bg-sp-hover text-sp-text-secondary"
                    )}
                  >
                    <span className="flex items-center gap-2">
                      <span className="flex-1 truncate text-[13px] font-medium text-sp-text" title={title}>{title}</span>
                      <span className={cn("shrink-0 inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs", runStatusClass(item.run.status))}>
                        {isRunActive(item.run.status) && (
                          <span className="timeline-quiet-dots" aria-hidden="true">
                            <span />
                            <span />
                            <span />
                          </span>
                        )}
                        {runStatusLabel(item.run.status)}
                      </span>
                    </span>
                    <span className="flex flex-wrap items-center gap-x-2 text-xs text-sp-text-muted">
                      <span>{formatRunStarted(item.run.started_at)}</span>
                      {duration && <span>{duration}</span>}
                      {item.run.model_route && <span className="truncate font-mono" title={item.run.model_route}>{item.run.model_route}</span>}
                      <span>任务 {item.created_task_ids.length}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          {selected && (
            <section aria-label="运行详情" className="mx-2 mb-2 flex flex-1 min-h-0 flex-col gap-2 overflow-y-auto eo-inset-well px-3 py-3">
              {usage && <div className="text-[13px] text-sp-text-secondary">{usage}</div>}
              {rows.length === 0 ? (
                <EmptyState kind="all-done" size="row" title={t("agent.workPanel.emptyNoDownloads")} className="py-4" />
              ) : (
                <ul aria-label="创建的任务" className="flex flex-col gap-1">
                  {rows.map(({ task_id, task }) => {
                    const percent = taskProgressPercent(task);
                    const status = taskStatusLabel(task);
                    const name = task?.name || task_id;
                    const content = (
                      <>
                        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", taskDotClass(task))} aria-hidden="true" />
                        <span className="flex-1 min-w-0 truncate text-[13px] text-sp-text" title={name}>{name}</span>
                        {task && (
                          <span className="h-1 w-16 shrink-0 overflow-hidden rounded-full bg-sp-line" aria-hidden="true">
                            <span className="block h-full rounded-full bg-sp-accent" style={{ width: `${percent ?? 0}%` }} />
                          </span>
                        )}
                        {percent !== null && <span className="w-10 shrink-0 text-right font-mono tabular-nums text-xs text-sp-text-muted">{percent}%</span>}
                        <span className="w-12 shrink-0 text-right text-xs text-sp-text-muted">{status}</span>
                      </>
                    );
                    return (
                      <li key={task_id}>
                        {task && onOpenTask ? (
                          <button
                            type="button"
                            onClick={() => onOpenTask(task_id)}
                            aria-label={`${name} ${status}`}
                            title="在下载中心查看"
                            className="group flex h-9 w-full items-center gap-2 rounded-sp-sm bg-sp-surface px-2.5 text-left transition-colors duration-fast ease-sp hover:bg-sp-hover cursor-pointer outline-none focus-visible:ring-1 focus-visible:ring-sp-accent"
                          >
                            {content}
                            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" aria-hidden="true" />
                          </button>
                        ) : (
                          <div className="flex h-9 items-center gap-2 rounded-sp-sm bg-sp-surface/60 px-2.5">{content}</div>
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>
          )}
        </>
      )}
    </div>
  );
}
