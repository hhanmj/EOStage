/**
 * AgentConclusionCard.tsx
 * Agent 结论卡组件：
 * - 顶部：思考过程折叠条 / 本轮思考用时标题
 * - 中部：Markdown 结论正文 + 可选关联任务卡 +（仅当存在真实引用凭据时）精简引用溯源折叠行
 * - 底部：纯图标单行操作栏（复制、重试、悬停用量图标）
 */

import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { ArrowUpRight, FileText, CheckCircle2, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { StreamConclusionItem } from "@/shell/api/agentStreamLogic";
import type { ApplicationTask } from "@/shell/api/types";
import {
  projectEvidence,
  buildTaskCardModels,
  evidenceSummary,
} from "@/shell/api/agentConversationLogic";
import { splitConclusionJsonBlock } from "@/shell/api/conclusionBodyLogic";
import { splitInlineThink } from "@/shell/api/agentThinkSplitLogic";
import { HoverHintLabel } from "./HoverHintLabel";
import { ChannelReasoningNote } from "@/shell/components/settings/ChannelReasoningNote";
import { AgentTaskCard } from "./AgentTaskCard";
import { AgentMarkdownRenderer } from "./agent/AgentMarkdownRenderer";
import { AgentThinkingCard } from "./agent/AgentThinkingCard";
import { AgentMessageActionBar } from "./agent/AgentMessageActionBar";
import type { RunUsageDetailState } from "./agentRunUsageHooks";

export interface AgentConclusionCardProps {
  item: StreamConclusionItem;
  agentTasks?: ApplicationTask[];
  isEvidenceExpanded?: boolean;
  onToggleEvidenceExpand?: (runId: string) => void;
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "cancel" | "retry"
  ) => Promise<void>;
  showActionBar?: boolean;
  turnIndex?: number;
  isRunning?: boolean;
  onRetryTurn?: (runId?: string, turnIndex?: number) => void | Promise<void>;
  onStopRun?: (runId?: string) => void | Promise<void>;
  onCopy?: (cleanText: string) => void;
  fallbackDurationMs?: number | null;
  suppressThinkingHeader?: boolean;
  usageTotal?: unknown;
  usageDetail?: RunUsageDetailState;
  onRequestUsageDetail?: () => void;
  batchLabelsById?: Readonly<Record<string, string>>;
  onSelectBatchRef?: (batchId: string) => void;
}

const RAW_BATCH_UUID_RE = /^(?:批次\s*#?)?batch-[0-9a-f-]{16,}$/i;
const RAW_TASK_UUID_RE = /^(?:任务\s*#?)?task-[0-9a-f-]{12,}$/i;

function formatHumanEvidenceRefLabel(
  rf: { kind: string; id: string; label: string; responseZh: string | null },
  index: number,
  batchLabelsById: Readonly<Record<string, string>> | undefined,
  agentTasks: ApplicationTask[]
): string {
  const rawLabel = (rf.label || "").trim();
  if (rf.kind === "batch") {
    const mapped = batchLabelsById?.[rf.id]?.trim();
    if (mapped) return mapped;
    if (rawLabel && !RAW_BATCH_UUID_RE.test(rawLabel)) {
      return rawLabel;
    }
    return rf.responseZh
      ? `数据检索记录 #${index + 1}（${rf.responseZh}）`
      : `数据检索记录 #${index + 1}`;
  }
  if (rf.kind === "task") {
    const matched = agentTasks.find(
      (tk) => tk.task_id === rf.id || (tk as { id?: string }).id === rf.id
    );
    if (matched?.name?.trim()) {
      return matched.name.trim();
    }
    if (rawLabel && !RAW_TASK_UUID_RE.test(rawLabel)) {
      return rawLabel;
    }
    return `下载任务记录 #${index + 1}`;
  }
  return rawLabel || rf.id;
}

export function AgentConclusionCard({
  item,
  agentTasks = [],
  isEvidenceExpanded: controlledExpanded,
  onToggleEvidenceExpand,
  onTaskAction,
  showActionBar = false,
  turnIndex,
  isRunning = false,
  onRetryTurn,
  onStopRun,
  onCopy,
  fallbackDurationMs = null,
  suppressThinkingHeader = false,
  usageTotal,
  usageDetail,
  onRequestUsageDetail,
  batchLabelsById,
  onSelectBatchRef,
}: AgentConclusionCardProps) {
  const { t } = useI18n();
  const [internalExpanded, setInternalExpanded] = useState(false);
  const isExpanded =
    controlledExpanded !== undefined ? controlledExpanded : internalExpanded;
  const toggleExpand = onToggleEvidenceExpand
    ? () => onToggleEvidenceExpand(item.run_id)
    : () => setInternalExpanded((v) => !v);

  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const inlineSplit = splitInlineThink(item.text);
  const { body: conclusionBody, jsonBlocks } = splitConclusionJsonBlock(item.text);
  const resolvedThinkingText =
    typeof item.thinking_text === "string" && item.thinking_text.trim().length > 0
      ? item.thinking_text
      : inlineSplit.thinkingText;

  const effectiveDurationMs =
    typeof item.thinking_ms === "number" && Number.isFinite(item.thinking_ms) && item.thinking_ms > 0
      ? item.thinking_ms
      : fallbackDurationMs;

  const projEvidence = projectEvidence(item);
  const displayEvidenceRefs = useMemo(() => {
    const seen = new Set<string>();
    const out: typeof projEvidence.refs = [];
    for (const rf of projEvidence.refs) {
      const dedupeKey = `${rf.kind}::${rf.id}::${rf.label}`;
      if (seen.has(dedupeKey)) continue;
      seen.add(dedupeKey);
      out.push(rf);
    }
    return out;
  }, [projEvidence.refs]);
  const evidenceListRef = useRef<HTMLDivElement | null>(null);
  const prevEvidenceExpandedRef = useRef(false);
  useEffect(() => {
    const wasExpanded = prevEvidenceExpandedRef.current;
    prevEvidenceExpandedRef.current = isExpanded;
    if (!wasExpanded && isExpanded) {
      const target = evidenceListRef.current;
      if (target && typeof target.scrollIntoView === "function") {
        target.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
    }
  }, [isExpanded]);
  const summary = evidenceSummary(item);
  const summaryHintId = useId();
  const summaryHint = summary.status === "verified" ? null : summary.reasonZh;
  const taskRefs = projEvidence.refs.filter((r) => r.kind === "task");
  const taskModels = buildTaskCardModels(taskRefs, agentTasks);
  const shouldRenderActionBar =
    showActionBar || Boolean(onRetryTurn || onStopRun || usageTotal);

  return (
    <div
      id="agent-conclusion-card"
      data-testid="eo-conclusion-card"
      className="group flex flex-col gap-2.5 w-full select-text"
    >
      {/* 顶部：思考过程折叠条 / 本轮思考耗时 */}
      {!suppressThinkingHeader && (
        <AgentThinkingCard
          status="done"
          thinkingText={resolvedThinkingText}
          durationMs={effectiveDurationMs}
          showElapsedWithoutText
        />
      )}

      {/* 结论 Markdown 正文 */}
      <AgentMarkdownRenderer content={conclusionBody} />

      {/* 折叠的重复 JSON 代码块 */}
      {jsonBlocks.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <button
            type="button"
            aria-expanded={detailsExpanded}
            onClick={() => setDetailsExpanded((v) => !v)}
            className="flex items-center gap-1.5 text-xs text-sp-text-secondary hover:text-sp-text cursor-pointer w-fit select-none"
          >
            <ChevronRight
              className={cn(
                "h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp",
                detailsExpanded && "rotate-90"
              )}
            />
            <span>{t(detailsExpanded ? "agent.conclusion.collapseDetails" : "agent.conclusion.expandDetails")}</span>
          </button>
          {detailsExpanded && (
            <div className="flex flex-col gap-2">
              {jsonBlocks.map((block, bIdx) => (
                <pre
                  key={bIdx}
                  className="font-mono text-xs text-sp-text-muted bg-sp-canvas border border-sp-line rounded-sp-sm p-2 overflow-x-auto whitespace-pre-wrap break-all m-0 select-text"
                >
                  {block}
                </pre>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 仅当存在结构化陈述（status !== "uncited"）时展示轻量核验脚标；彻底隐藏裸 UUID 乱码并支持一键切到对应批次 */}
      {projEvidence.status !== "uncited" && (
        <div className="flex flex-col gap-1.5 pt-0.5">
          {projEvidence.refCount > 0 ? (
            <>
              <button
                type="button"
                onClick={toggleExpand}
                aria-expanded={isExpanded}
                aria-describedby={summaryHint ? summaryHintId : undefined}
                className="group/evidence flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-sp-text-muted hover:text-sp-text cursor-pointer w-fit select-none transition-colors duration-fast ease-sp"
              >
                <FileText className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
                <span>{t("agent.conclusion.evidence")}</span>
                <EvidenceSummaryBadge summary={summary} parentTooltipId={summaryHintId} />
                <span className="text-sp-text-muted">
                  {t("agent.conclusion.referenceCount", { count: displayEvidenceRefs.length })}
                </span>
                <ChevronRight
                  className={cn(
                    "h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp",
                    isExpanded && "rotate-90"
                  )}
                />
              </button>

              {isExpanded && (
                <div ref={evidenceListRef} className="pl-5 space-y-1 py-0.5 select-text">
                  {displayEvidenceRefs.map((rf, rIdx) => {
                    const humanLabel = formatHumanEvidenceRefLabel(
                      rf,
                      rIdx,
                      batchLabelsById,
                      agentTasks
                    );
                    const canJumpBatch = Boolean(onSelectBatchRef && rf.kind === "batch" && rf.id);
                    return (
                      <div
                        key={rIdx}
                        className="flex items-center justify-between rounded-lg bg-sp-card/80 border border-sp-line px-2.5 py-1.5 text-xs"
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="px-1.5 py-0.5 rounded bg-sp-surface-muted text-xs font-medium text-sp-text-secondary border border-sp-line shrink-0">
                            {rf.kind_zh}
                          </span>
                          {canJumpBatch ? (
                            <button
                              type="button"
                              onClick={() => onSelectBatchRef?.(rf.id)}
                              className="inline-flex items-center gap-1 min-w-0 text-xs font-medium text-sp-text hover:text-sp-accent cursor-pointer transition-colors"
                            >
                              <span className="truncate">{humanLabel}</span>
                              <ArrowUpRight className="h-3 w-3 shrink-0 text-sp-text-muted" />
                            </button>
                          ) : (
                            <span className="text-xs font-medium text-sp-text truncate select-text">
                              {humanLabel}
                            </span>
                          )}
                        </div>
                        <div className="flex items-center gap-2 shrink-0 ml-2">
                          <HoverHintLabel
                            hint={rf.reasonZh}
                            focusable
                            align="end"
                            className={cn(
                              "text-xs font-semibold",
                              rf.verified ? "text-sp-success-text" : "text-sp-text-muted"
                            )}
                          >
                            {rf.verified === true
                              ? t("agent.conclusion.verified")
                              : rf.verified === false
                                ? t("agent.conclusion.unverified")
                                : t("agent.conclusion.unknown")}
                          </HoverHintLabel>
                        </div>
                      </div>
                    );
                  })}
                  {summary.uncitedClaims > 0 && (
                    <div className="rounded-lg bg-sp-card/80 border border-sp-line px-2.5 py-1.5 text-xs text-sp-text-muted">
                      {t("agent.conclusion.uncitedClaims", { count: summary.uncitedClaims })}
                    </div>
                  )}
                </div>
              )}
            </>
          ) : (
            <div className="flex items-center gap-2 text-xs text-sp-text-muted select-none">
              <FileText className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
              <span>{t("agent.conclusion.evidence")}</span>
              <EvidenceSummaryBadge summary={summary} focusable />
            </div>
          )}
        </div>
      )}

      {/* 思考被拒渠道提示 */}
      <ChannelReasoningNote refused={item.reasoning_off_refused} />

      {/* 关联任务卡 */}
      {taskModels.length > 0 && (
        <div className="space-y-3 pt-1">
          {taskModels.map((m) => (
            <AgentTaskCard
              key={m.taskId}
              model={m}
              onTaskAction={onTaskAction}
            />
          ))}
        </div>
      )}

      {/* 助手轮次单行纯图标操作栏（复制 · 重试 · 悬停用量图标） */}
      {shouldRenderActionBar && (
        <AgentMessageActionBar
          role="assistant"
          content={conclusionBody}
          runId={item.run_id}
          turnIndex={turnIndex}
          isRunning={isRunning}
          onRetryTurn={onRetryTurn}
          onStopRun={onStopRun}
          onCopy={onCopy}
          usageTotal={usageTotal}
          usageDetail={usageDetail}
          onRequestUsageDetail={onRequestUsageDetail}
        />
      )}
    </div>
  );
}

function EvidenceSummaryBadge({
  summary,
  focusable = false,
  parentTooltipId,
}: {
  summary: ReturnType<typeof evidenceSummary>;
  focusable?: boolean;
  parentTooltipId?: string;
}) {
  if (summary.status === "verified") {
    return (
      <span className="inline-flex items-center gap-1 text-sp-success-text font-semibold">
        <CheckCircle2 className="h-3.5 w-3.5 text-sp-status-success" />
        <span>{summary.labelZh}</span>
      </span>
    );
  }
  return (
    <HoverHintLabel
      hint={summary.reasonZh}
      focusable={focusable}
      parentTooltipId={parentTooltipId}
      className={cn(
        "font-semibold",
        summary.status === "partial" ? "text-sp-warning-text" : "text-sp-text-muted"
      )}
    >
      {summary.labelZh}
    </HoverHintLabel>
  );
}
