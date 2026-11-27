/**
 * AgentPlanCard.tsx
 * Agent 计划卡独立组件（支持自动执行徽章、模式名称、专属工具语义图标与步骤检索披露）
 * 规范：v3-contracts.ts、EOStage 权限三档规范
 */

import React from "react";
import { Globe, CheckCircle2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { StreamPlanCardItem } from "../../shell/api/agentStreamLogic";
import { projectApproval } from "../../shell/api/approvalLogic";
import { disclosureLine } from "../../shell/api/planDisclosure";
import {
  projectPlanStepStatus,
  planStepStatusLabelZh,
} from "../../shell/api/agentConversationLogic";
import { renderToolSemanticGlyph } from "./agent/agentToolSemantics";

export interface AgentPlanCardProps {
  item: StreamPlanCardItem;
  sourceNames?: Readonly<Record<string, string>>;
  progressStatus?: string | null;
  progressCode?: string | null;
}

export function AgentPlanCard({
  item,
  sourceNames,
  progressStatus = null,
  progressCode = null,
}: AgentPlanCardProps) {
  const approvalProj = projectApproval(
    item.approval,
    item.auto_executed
  );
  const isAuto = approvalProj.kind === "auto";
  const isRunning =
    progressStatus === "running" ||
    item.steps.some(
      (st) => projectPlanStepStatus(st.status, progressStatus, progressCode) === "running"
    );

  return (
    <div
      data-testid="eo-plan-card"
      className={cn(
        "cds-decision-card",
        isRunning ? "eo-concentric-band eo-artifact-card-pulse" : "eo-concentric-neutral",
        "w-full"
      )}
      aria-label="计划卡"
    >
      {/* Concentric Top Eyebrow Header */}
      <div className="cds-decision-header flex items-center gap-2 px-3 py-1.5">
        <span
          aria-hidden="true"
          className={cn(
            "h-2 w-2 rounded-full shrink-0",
            isRunning ? "bg-sp-accent" : "bg-sp-status-success"
          )}
        />
        <span className="text-xs font-semibold text-sp-text tracking-tight">
          遥感检索与执行计划
        </span>
        <span className="text-xs font-mono text-sp-text-muted">
          ({item.steps.length} 步)
        </span>
        {isAuto && (
          <div className="ml-auto flex items-center gap-1.5">
            <span className="rounded-full bg-sp-card text-sp-text-secondary border border-sp-line px-2 py-0.5 text-xs font-medium">
              已自动执行
            </span>
            {approvalProj.modeLabel && (
              <span className="rounded-full bg-sp-card text-sp-text-secondary border border-sp-line px-2 py-0.5 text-xs font-medium">
                {approvalProj.modeLabel}
              </span>
            )}
          </div>
        )}
      </div>

      {/* Concentric Inner Surface */}
      <div className={cn("eo-concentric-inner p-3 space-y-1", isRunning && "eo-artifact-card-pulse")}>
        {item.steps.map((st, sIdx) => {
          const disclosureText = disclosureLine(st.disclosure, sourceNames);
          const effectiveStatus = projectPlanStepStatus(
            st.status,
            progressStatus,
            progressCode
          );
          const statusLabel = planStepStatusLabelZh(effectiveStatus);
          return (
            <div
              key={sIdx}
              data-tool-id={st.tool_id}
              className={cn(
                "sp-row-balanced flex items-start justify-between gap-2 rounded-[7px] px-2.5 text-xs transition-colors hover:bg-sp-hover",
                effectiveStatus === "running" && "sp-task-shimmer-line"
              )}
            >
              <div className="flex items-start gap-2 min-w-0 flex-1">
                <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center">
                  {renderToolSemanticGlyph(st.tool_id, {
                    status: effectiveStatus,
                    className: "h-3.5 w-3.5 shrink-0",
                  })}
                </span>
                <div className="flex flex-col gap-0.5 min-w-0 flex-1">
                  <span className="text-sp-text font-medium break-words">
                    {st.label_zh}
                  </span>
                  {disclosureText && (
                    <div className="flex items-center gap-1 text-xs text-sp-text-muted break-words">
                      <Globe className="h-3 w-3 shrink-0" aria-hidden="true" />
                      <span>{disclosureText}</span>
                    </div>
                  )}
                </div>
              </div>
              <span
                className={cn(
                  "inline-flex items-center gap-1 text-xs shrink-0 whitespace-nowrap tabular-nums",
                  effectiveStatus === "done"
                    ? "text-sp-success-text font-medium"
                    : effectiveStatus === "running"
                    ? "text-sp-accent font-medium"
                    : "text-sp-text-muted"
                )}
              >
                {effectiveStatus === "done" && (
                  <CheckCircle2 className="h-3.5 w-3.5 text-sp-status-success shrink-0" aria-hidden="true" />
                )}
                {effectiveStatus === "running" && (
                  <span
                    data-testid="plan-step-quiet-dots"
                    className="timeline-quiet-dots"
                    aria-hidden="true"
                  >
                    <span />
                    <span />
                    <span />
                  </span>
                )}
                <span>{statusLabel}</span>
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
