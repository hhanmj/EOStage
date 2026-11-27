import React, { useState } from "react";
import { ChevronDown, ChevronRight, CheckCircle2, AlertCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AgentRunSummary } from "../../lib/generated/v3-contracts.ts";
import { runStatusLabel } from "../../shell/api/agentStreamLogic.ts";
import {
  formatRunStarted,
  runBadgeFlags,
  runBadgeLabels,
} from "../../shell/api/agentRunBadges.ts";
import { runUsageText } from "../../shell/api/agentUsage.ts";

export interface AgentRunListStripProps {
  runs: AgentRunSummary[];
  defaultExpanded?: boolean;
  /** 当作为 Composer 顶部同心圆 InputShellEyebrow 呈现时使用紧凑眉栏样式 */
  variant?: "inline" | "eyebrow";
}

export function AgentRunListStrip({
  runs,
  defaultExpanded = false,
  variant = "inline",
}: AgentRunListStripProps) {
  const [expanded, setExpanded] = useState(defaultExpanded);

  if (!Array.isArray(runs) || runs.length === 0) {
    return null;
  }

  const Chevron = expanded ? ChevronDown : ChevronRight;
  const completedCount = runs.filter((r) => r.status === "completed").length;
  const hasRunning = runs.some((r) => r.status === "running");
  const hasFailed = runs.some((r) => r.status === "failed");

  return (
    <div
      data-testid="task-tracker-eyebrow"
      data-variant={variant}
      className={cn(
        "w-full flex flex-col transition-colors duration-fast ease-sp",
        variant === "eyebrow" ? "px-2 py-0.5" : "gap-1.5 py-1"
      )}
    >
      <div className="flex h-7 items-center justify-between gap-2">
        <button
          type="button"
          aria-label={`运行记录 ${runs.length}`}
          aria-expanded={expanded}
          title="运行记录"
          onClick={() => setExpanded((prev) => !prev)}
          className="flex flex-1 min-w-0 items-center gap-2 text-xs text-sp-text-secondary hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer select-none outline-none"
        >
          {hasRunning ? (
            <span
              data-testid="run-strip-quiet-dots"
              className="timeline-quiet-dots"
              aria-hidden="true"
            >
              <span />
              <span />
              <span />
            </span>
          ) : hasFailed ? (
            <AlertCircle className="h-3.5 w-3.5 shrink-0 text-sp-danger-text" aria-hidden="true" />
          ) : (
            <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-sp-status-success" aria-hidden="true" />
          )}
          <span className="font-medium text-sp-text">
            运行记录 {runs.length}
          </span>
          <span
            data-testid="task-tracker-eyebrow-counter"
            className="ml-auto shrink-0 font-mono text-xs text-sp-text-muted tabular-nums"
          >
            {completedCount} / {runs.length}
          </span>
          <Chevron className="h-3.5 w-3.5 shrink-0 text-sp-text-muted transition-transform duration-fast" />
        </button>
      </div>

      {expanded && (
        <div
          data-testid="task-tracker-eyebrow-rows"
          className="mt-1 flex flex-col gap-1 border-t border-sp-divider pt-1.5 pb-1 pl-1"
        >
          {runs.map((run, idx) => {
            const flags = runBadgeFlags(run);
            const badges = runBadgeLabels(flags);
            const statusText = runStatusLabel(run.status);
            const formattedTime = formatRunStarted(run.started_at);
            const usageText = runUsageText(run.usage_total);
            const isRunning = run.status === "running";
            const isCompleted = run.status === "completed";
            const isFailed = run.status === "failed";

            return (
              <div
                key={run.run_id || idx}
                data-testid="task-tracker-eyebrow-row"
                className={cn(
                  "sp-row-compact flex items-center gap-2 rounded-lg px-2 text-xs min-w-0 whitespace-nowrap overflow-hidden transition-colors",
                  isRunning
                    ? "bg-sp-selected text-sp-accent"
                    : "text-sp-text-secondary hover:bg-sp-hover"
                )}
              >
                {isCompleted ? (
                  <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-sp-status-success" aria-hidden="true" />
                ) : isRunning ? (
                  <span className="timeline-quiet-dots" aria-hidden="true">
                    <span />
                    <span />
                    <span />
                  </span>
                ) : isFailed ? (
                  <AlertCircle className="h-3.5 w-3.5 shrink-0 text-sp-danger-text" aria-hidden="true" />
                ) : (
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-sp-text-muted" aria-hidden="true" />
                )}
                <span className="font-medium text-sp-text shrink-0">
                  {statusText}
                </span>
                <span
                  className="truncate text-sp-text-muted tabular-nums"
                  title={formattedTime}
                >
                  {formattedTime}
                </span>
                {badges.map((label) => (
                  <span
                    key={label}
                    className="shrink-0 rounded-full bg-sp-surface-muted text-sp-text-secondary border border-sp-line px-2 py-0.5 text-xs font-semibold"
                  >
                    {label}
                  </span>
                ))}
                {usageText && (
                  <span className="ml-auto min-w-0 truncate text-sp-text-muted tabular-nums" title={usageText}>
                    {usageText}
                  </span>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
