/**
 * AgentToolGroupCard.tsx
 * 执行过程语义工具流组件：
 * - 统一接入 `agentToolSemantics` 4 类语义图标与人话简短说明（覆盖全部 18 核心工具 + GEE + GIS + 技能 + MCP）；
 * - 折叠态呈现关键空间/检索/任务胶囊（支持定位地图 / 在地图查看）；
 * - 展开态呈现纵向连续导轨时间线（节点图标 + 简短说明 + 单步耗时 + 完成对勾自绘，点击单步可展开完整摘要）。
 */

import React, { useEffect, useMemo, useRef, useState } from "react";
import { ChevronRight, Eye, Loader2, MapPin } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { AgentToolRoundPayload } from "@/lib/generated/v3-contracts";
import type { StreamToolGroupItem } from "@/shell/api/agentStreamLogic";
import { resolveAgentToolDisplayName } from "@/shell/api/agentLogic";
import {
  projectToolGroup,
  type ProjectToolGroupOptions,
} from "@/shell/api/agentConversationLogic";
import {
  getWorkStepsPrefs,
  shouldExpandToolGroupByDefault,
  subscribeWorkStepsPrefs,
  type AgentStepDetailMode,
  type WorkStepsCollapseTiming,
} from "@/lib/performanceAndUsagePrefs";
import {
  renderToolSemanticGlyph,
  resolveToolSemanticMeta,
  EO_MONOCHROME_ICON_CLASS,
} from "./agent/agentToolSemantics";
import { formatThinkingElapsed, renderWidthLockedTimerText } from "./agent/AgentThinkingCard";

const LOCATABLE_TOOL_IDS = new Set([
  "working_aoi_set",
  "admin_boundary_search",
  "map_workspace_action",
]);

const VIEWABLE_SEARCH_TOOL_IDS = new Set([
  "search_progress",
  "search_start",
  "workbench_page",
  "batch_page",
]);

const PILL_HIGHLIGHT_TOOL_IDS = new Set([
  "working_aoi_set",
  "admin_boundary_search",
  "search_start",
  "search_progress",
  "map_workspace_action",
  "create_task",
  "nisar_create_subset_task",
  "gee_cloud_analyze",
  "gee_chunked_download",
  "gis_local_process",
]);

export interface AgentToolGroupCardProps {
  item: StreamToolGroupItem;
  options?: ProjectToolGroupOptions;
  resolveToolName?: (toolId: string) => string;
  isExpanded?: boolean;
  defaultExpanded?: boolean;
  /** 嵌入在轮次统一「思考中 / 已思考」时间线内时隐藏二级「执行过程 N 步」外壳，直接平铺工具步骤行 */
  headless?: boolean;
  onToggleExpand?: () => void;
  stepDetailMode?: AgentStepDetailMode;
  collapseTiming?: WorkStepsCollapseTiming;
  hasSubsequentUserInput?: boolean;
  onOpenWorkStepsSettings?: () => void;
  onLocateOnMap?: (round: AgentToolRoundPayload) => void;
  onViewOnMap?: (round: AgentToolRoundPayload) => void;
}

export function AgentToolGroupCard({
  item,
  options,
  resolveToolName,
  isExpanded: controlledExpanded,
  defaultExpanded = false,
  headless = false,
  onToggleExpand,
  stepDetailMode: propStepMode,
  collapseTiming: propCollapseTiming,
  hasSubsequentUserInput = false,
  onLocateOnMap,
  onViewOnMap,
}: AgentToolGroupCardProps) {
  const { t } = useI18n();
  const [workStepsPrefs, setWorkStepsPrefsState] = useState(() => getWorkStepsPrefs());
  const [userToggled, setUserToggled] = useState(false);
  const [expandedRowIndices, setExpandedRowIndices] = useState<Set<number>>(new Set());

  const effectiveStepMode = propStepMode ?? workStepsPrefs.stepDetailMode;
  const effectiveCollapseTiming = propCollapseTiming ?? workStepsPrefs.collapseTiming;
  const isDetailed = effectiveStepMode === "detailed";

  const autoExpanded = shouldExpandToolGroupByDefault({
    stepDetailMode: effectiveStepMode,
    collapseTiming: effectiveCollapseTiming,
    itemStatus: item.status,
    hasSubsequentUserInput,
    defaultExpanded,
  });

  const [internalExpanded, setInternalExpanded] = useState(autoExpanded);

  useEffect(() => {
    return subscribeWorkStepsPrefs((prefs) => {
      setWorkStepsPrefsState((prev) => {
        if (
          prev.stepDetailMode !== prefs.stepDetailMode ||
          prev.collapseTiming !== prefs.collapseTiming
        ) {
          setUserToggled(false);
          setInternalExpanded(
            shouldExpandToolGroupByDefault({
              stepDetailMode: propStepMode ?? prefs.stepDetailMode,
              collapseTiming: propCollapseTiming ?? prefs.collapseTiming,
              itemStatus: item.status,
              hasSubsequentUserInput,
              defaultExpanded,
            })
          );
        }
        return prefs;
      });
    });
  }, [defaultExpanded, hasSubsequentUserInput, item.status, propCollapseTiming, propStepMode]);

  const isExpanded = headless
    ? true
    : controlledExpanded !== undefined
      ? controlledExpanded
      : userToggled
        ? internalExpanded
        : autoExpanded;

  const toggle =
    onToggleExpand ??
    (() => {
      setUserToggled(true);
      setInternalExpanded(!isExpanded);
    });

  const toggleRowDetail = (idx: number) => {
    setExpandedRowIndices((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  const effectiveResolveToolName = resolveToolName ?? resolveAgentToolDisplayName;
  const proj = projectToolGroup(item, effectiveResolveToolName, options);
  const rawRounds = item.rounds || [];

  // 防闪烁对勾守卫：仅在步骤由 running 实时跃迁为 done 时播放一次 sp-check-draw；展开已完成折叠条时保持静止不重播
  const prevRowStatusesRef = useRef<Map<string, string>>(new Map());
  const freshlyDoneRowKeys = useMemo(() => {
    const fresh = new Set<string>();
    proj.rows.forEach((row, idx) => {
      const key = rawRounds[idx]?.call_id || `row-${idx}`;
      const prev = prevRowStatusesRef.current.get(key);
      if (prev !== undefined && prev !== "done" && row.status === "done") {
        fresh.add(key);
      }
    });
    return fresh;
  }, [proj.rows, rawRounds]);

  useEffect(() => {
    const next = new Map<string, string>();
    proj.rows.forEach((row, idx) => {
      const key = rawRounds[idx]?.call_id || `row-${idx}`;
      next.set(key, row.status);
    });
    prevRowStatusesRef.current = next;
  }, [proj.rows, rawRounds]);
  const runningRowIdx = proj.rows.findIndex((r) => r.status === "running");
  const hasRunningRow = runningRowIdx >= 0;
  const isRunningGroup = item.status === "running" || hasRunningRow;
  const failedCount = proj.rows.filter((r) => r.status === "failed").length;

  const totalDurationMs = rawRounds.reduce(
    (sum, r) => (typeof r?.duration_ms === "number" && r.duration_ms > 0 ? sum + r.duration_ms : sum),
    0
  );
  const totalDurationLabel =
    !isRunningGroup && totalDurationMs > 0
      ? totalDurationMs >= 1000
        ? formatThinkingElapsed(totalDurationMs, t, false)
        : t("agent.conversation.milliseconds", { count: Math.round(totalDurationMs) })
      : null;

  const activeStepHint =
    hasRunningRow && proj.rows[runningRowIdx]
      ? proj.rows[runningRowIdx].name_zh
      : null;

  const pillEntries = proj.rows
    .map((row, rIdx) => ({ row, rawRound: rawRounds[rIdx], rIdx }))
    .filter(
      ({ row, rawRound }) =>
        rawRound &&
        (PILL_HIGHLIGHT_TOOL_IDS.has(rawRound.tool_id) ||
          row.status === "failed" ||
          rawRound.result_count === 0)
    );

  return (
    <div
      data-testid="agent-tool-group-card"
      data-step-detail-mode={effectiveStepMode}
      data-collapse-timing={effectiveCollapseTiming}
      data-expanded={isExpanded ? "true" : "false"}
      data-headless={headless ? "true" : "false"}
      className="sp-tool-group-card group/tool-card flex flex-col gap-1 w-full select-text"
    >
      {/* 顶部单行静默摘要头（仅独立渲染时显示；嵌入统一思考时间线时由顶部「思考中 / 已思考」统一收纳） */}
      {!headless && (
        <div className="flex items-center justify-between gap-2 min-h-[28px]">
          <button
            type="button"
            onClick={toggle}
            aria-expanded={isExpanded}
            className="cds-btn-squish flex items-center gap-2 h-7 px-2 -ml-2 rounded-md text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer max-w-full select-none"
          >
            <ChevronRight
              className={cn(
                "h-3.5 w-3.5 text-sp-text-muted transition-transform duration-fast ease-sp shrink-0",
                isExpanded && "rotate-90"
              )}
            />
            {isRunningGroup && (
              <span
                data-testid="tool-group-quiet-dots"
                data-glyph-mode="circuit"
                className="timeline-quiet-dots"
                aria-hidden="true"
              >
                <span />
                <span />
                <span />
              </span>
            )}
            <span className="font-medium shrink-0">
              {t("agent.toolSteps.groupTitle", { count: proj.count })}
            </span>
            {activeStepHint ? (
              <span className="truncate text-sp-text-muted">· 正在执行：{activeStepHint}</span>
            ) : totalDurationLabel ? (
              <span className="shrink-0 tabular-nums text-sp-text-muted">
                · {renderWidthLockedTimerText(totalDurationLabel)}
              </span>
            ) : null}
            {failedCount > 0 && (
              <span className="shrink-0 text-xs text-sp-text-muted">
                · {failedCount} 项失败
              </span>
            )}
            <span className="shrink-0 text-sp-text-muted">
              {isExpanded ? t("agent.toolSteps.expanded") : t("agent.toolSteps.collapsed")}
            </span>
            {isDetailed && (
              <span className="shrink-0 rounded-sp-sm bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 text-xs text-sp-text-secondary">
                {t("agent.toolSteps.badge")}
              </span>
            )}
          </button>
        </div>
      )}

      {/* 折叠态：关键空间 / 检索 / 交付胶囊 */}
      {!headless && !isExpanded && pillEntries.length > 0 && (
        <div
          data-testid="agent-tool-pills-strip"
          className="flex flex-wrap items-center gap-1.5 pl-5 py-0.5"
        >
          {pillEntries.map(({ row, rawRound, rIdx }) => {
            const toolId = rawRound.tool_id;
            const meta = resolveToolSemanticMeta(toolId);
            const canLocate = Boolean(onLocateOnMap && LOCATABLE_TOOL_IDS.has(toolId));
            const canView = Boolean(
              onViewOnMap &&
                VIEWABLE_SEARCH_TOOL_IDS.has(toolId) &&
                (typeof rawRound.result_count !== "number" || rawRound.result_count > 0)
            );
            return (
              <div
                key={`${rawRound.call_id || rIdx}`}
                data-testid="agent-tool-pill"
                data-tool-id={toolId}
                data-tool-status={row.status}
                className="inline-flex items-center gap-1.5 rounded-full border border-sp-line bg-sp-card/85 px-2.5 py-1 text-xs text-sp-text-secondary"
              >
                {renderToolSemanticGlyph(toolId, {
                  status: row.status,
                  resultCount: rawRound.result_count,
                })}
                <span className="font-medium text-sp-text">{row.name_zh}</span>
                <span className="max-w-[240px] truncate text-sp-text-muted">
                  {row.summaryZh || meta.defaultBriefZh}
                </span>
                {row.resultCountZh && (
                  <span className="text-xs text-sp-text-muted tabular-nums">
                    · {row.resultCountZh}
                  </span>
                )}
                {canLocate && (
                  <button
                    type="button"
                    data-testid="btn-tool-locate-map"
                    onClick={() => onLocateOnMap?.(rawRound)}
                    className="inline-flex items-center gap-1 rounded-full bg-sp-surface-muted px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer select-none"
                  >
                    <MapPin className={cn("h-3 w-3 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
                    <span>{t("agent.toolSteps.locateOnMap")}</span>
                  </button>
                )}
                {canView && (
                  <button
                    type="button"
                    data-testid="btn-tool-view-map"
                    onClick={() => onViewOnMap?.(rawRound)}
                    className="inline-flex items-center gap-1 rounded-full bg-sp-surface-muted px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer select-none"
                  >
                    <Eye className={cn("h-3 w-3 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
                    <span>{t("agent.toolSteps.viewOnMap")}</span>
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}

      {!headless && !isExpanded && proj.count > 0 && (
        <div
          aria-hidden="true"
          data-testid="agent-tool-folder-peek"
          className="sp-tool-folder-peek"
        />
      )}

      {/* 展开态：EOStage 单色裸线条图标对齐轴 + 简短说明 + 单步耗时 */}
      {isExpanded && (
        <div
          data-testid="agent-tool-drawer"
          className={cn("flex items-stretch", !headless && "pl-1 pt-0.5")}
        >
          {!headless && (
            <button
              type="button"
              data-testid="eo-tool-collapse-rail"
              aria-label={t("agent.toolSteps.collapseRailAria")}
              onClick={toggle}
              className="group/collapse-rail flex w-3 shrink-0 cursor-pointer items-stretch justify-start py-1 outline-none"
            >
              <span className="eo-collapse-rail h-full" />
            </button>
          )}

          <div className="min-w-0 flex-1 space-y-0.5">
            {proj.rows.map((row, rIdx) => {
              const rawRound = rawRounds[rIdx];
              const toolId = rawRound?.tool_id;
              const meta = resolveToolSemanticMeta(toolId);
              const isRowDetailOpen = isDetailed || expandedRowIndices.has(rIdx);
              const displayBrief = row.summaryZh || meta.defaultBriefZh;
              const rawRec = rawRound as Record<string, unknown> | undefined;
              const targetLabel =
                row.targetLabel ??
                (typeof rawRec?.target_label === "string" && rawRec.target_label.trim()
                  ? rawRec.target_label.trim()
                  : null);
              const inputSummary =
                row.inputSummary ??
                (typeof rawRec?.input_summary === "string" && rawRec.input_summary.trim()
                  ? rawRec.input_summary.trim()
                  : null);
              const outputSummary =
                row.outputSummary ??
                (typeof rawRec?.output_summary === "string" && rawRec.output_summary.trim()
                  ? rawRec.output_summary.trim()
                  : null);
              const hasExpandableDetail = Boolean(
                inputSummary ||
                  outputSummary ||
                  (row.summaryZh && row.summaryZh.trim().length > 0) ||
                  (displayBrief && displayBrief.trim().length > 0)
              );

              const canLocate = Boolean(
                onLocateOnMap && rawRound && toolId && LOCATABLE_TOOL_IDS.has(toolId)
              );
              const canView = Boolean(
                onViewOnMap &&
                  rawRound &&
                  toolId &&
                  VIEWABLE_SEARCH_TOOL_IDS.has(toolId) &&
                  (typeof rawRound.result_count !== "number" || rawRound.result_count > 0)
              );

              return (
                <div
                  key={rIdx}
                  data-testid="eo-chat-event-item"
                  data-tool-id={toolId}
                  data-tool-status={row.status}
                  style={{ "--sp-step-idx": rIdx } as React.CSSProperties}
                  className={cn(
                    "sp-tool-step-card relative flex items-start gap-2 rounded-md px-1.5 py-1 text-xs transition-colors duration-fast hover:bg-sp-hover/65",
                    row.status === "running" && "sp-task-shimmer-line bg-sp-hover/35"
                  )}
                >
                  {/* 左侧定宽制表耗时列（杜绝右端抖动，严格统一交互布局 竖向单色时间轴） */}
                  <span
                    data-testid="agent-tool-left-duration"
                    className="w-[58px] shrink-0 text-right font-mono text-xs tabular-nums text-sp-text-muted pt-[2px] select-none"
                  >
                    {row.durationZh
                      ? renderWidthLockedTimerText(row.durationZh)
                      : row.status === "running"
                        ? "…"
                        : ""}
                  </span>

                  {/* 左侧单色无底框 EOStage 语义图标轴（白底灰黑线 / 黑底灰白线） */}
                  <div className="flex w-4 shrink-0 items-center justify-center pt-[3px]">
                    {renderToolSemanticGlyph(toolId, {
                      status: row.status,
                      resultCount: rawRound?.result_count,
                      className: "h-3.5 w-3.5 shrink-0",
                    })}
                  </div>

                  {/* 右侧内容：工具名称 + 操作对象标签 + 简短说明 + 点击展开参数与结果抽屉 */}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center justify-between gap-2 min-h-[20px]">
                      {(() => {
                        const triggerInner = (
                          <>
                            <div className="flex items-center gap-1.5 min-w-0 flex-1">
                              <span className="font-medium text-sp-text-secondary shrink-0">
                                {row.name_zh}
                              </span>
                              {targetLabel && (
                                <code
                                  data-testid="agent-tool-target-label"
                                  title={targetLabel}
                                  className="max-w-[240px] truncate rounded bg-sp-card px-1.5 py-0.5 font-mono text-xs text-sp-text border border-sp-line/70 shrink-0"
                                >
                                  {targetLabel}
                                </code>
                              )}
                              {isDetailed && toolId && (
                                <code className="rounded bg-sp-surface-muted px-1.5 py-0.5 font-mono text-xs text-sp-text-muted shrink-0">
                                  {toolId}
                                </code>
                              )}
                              {!isRowDetailOpen && displayBrief && (
                                <span className="min-w-0 truncate text-sp-text-muted">
                                  {displayBrief}
                                </span>
                              )}
                              {row.status === "failed" && (
                                <span className="shrink-0 text-xs text-sp-text-muted underline decoration-sp-line underline-offset-2 select-none">
                                  {t("agent.status.failure")}
                                </span>
                              )}
                              {row.status === "running" && (
                                <span className="shrink-0 text-xs text-sp-text-muted select-none">
                                  {t("agent.status.running")}
                                </span>
                              )}
                            </div>

                            <div className="flex items-center gap-2 text-xs text-sp-text-muted shrink-0 tabular-nums">
                              {row.resultCountZh && (
                                <span className="text-sp-text-muted tabular-nums">
                                  {row.resultCountZh}
                                </span>
                              )}
                              {isDetailed && row.timeZh && <span>{row.timeZh}</span>}

                              {hasExpandableDetail && (
                                <ChevronRight
                                  className={cn(
                                    "h-3 w-3 text-sp-text-muted transition-transform duration-fast ease-sp shrink-0",
                                    isRowDetailOpen && "rotate-90"
                                  )}
                                />
                              )}

                              {(row.status === "running" || row.status === "done") && (
                                <span
                                  aria-hidden="true"
                                  data-testid="agent-tool-step-morph"
                                  data-state={row.status === "done" ? "done" : "running"}
                                  className="sp-action-icon-morph"
                                >
                                  <Loader2
                                    className={cn(
                                      "h-3.5 w-3.5 animate-spin",
                                      EO_MONOCHROME_ICON_CLASS,
                                      row.status === "running" ? "is-shown" : "is-hidden"
                                    )}
                                  />
                                  <svg
                                    viewBox="0 0 16 16"
                                    fill="none"
                                    className={cn(
                                      "h-3.5 w-3.5",
                                      EO_MONOCHROME_ICON_CLASS,
                                      row.status === "done" ? "is-shown" : "is-hidden"
                                    )}
                                  >
                                    <path
                                      d="M3.5 8.5L6.5 11.5L12.5 4.5"
                                      stroke="currentColor"
                                      strokeWidth="1.6"
                                      strokeLinecap="round"
                                      strokeLinejoin="round"
                                      className={
                                        row.status === "done" &&
                                        freshlyDoneRowKeys.has(rawRound?.call_id || `row-${rIdx}`)
                                          ? "sp-check-draw"
                                          : undefined
                                      }
                                    />
                                  </svg>
                                </span>
                              )}
                            </div>
                          </>
                        );
                        return hasExpandableDetail ? (
                          <button
                            type="button"
                            onClick={() => toggleRowDetail(rIdx)}
                            aria-expanded={isRowDetailOpen}
                            data-testid="agent-tool-row-trigger"
                            className="flex min-w-0 flex-1 items-center justify-between gap-2 text-left cursor-pointer focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent rounded"
                          >
                            {triggerInner}
                          </button>
                        ) : (
                          <div
                            data-testid="agent-tool-row-trigger"
                            className="flex min-w-0 flex-1 items-center justify-between gap-2"
                          >
                            {triggerInner}
                          </div>
                        );
                      })()}

                      {(canLocate || canView) && rawRound && (
                        <div className="flex items-center gap-1.5 shrink-0">
                          {canLocate && (
                            <button
                              type="button"
                              data-testid="btn-tool-locate-map"
                              onClick={(e) => {
                                e.stopPropagation();
                                onLocateOnMap?.(rawRound);
                              }}
                              className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-card px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer select-none"
                            >
                              <MapPin className={cn("h-3 w-3 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
                              <span>{t("agent.toolSteps.locateOnMap")}</span>
                            </button>
                          )}
                          {canView && (
                            <button
                              type="button"
                              data-testid="btn-tool-view-map"
                              onClick={(e) => {
                                e.stopPropagation();
                                onViewOnMap?.(rawRound);
                              }}
                              className="inline-flex items-center gap-1 rounded-full border border-sp-line bg-sp-card px-2 py-0.5 text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer select-none"
                            >
                              <Eye className={cn("h-3 w-3 shrink-0", EO_MONOCHROME_ICON_CLASS)} />
                              <span>{t("agent.toolSteps.viewOnMap")}</span>
                            </button>
                          )}
                        </div>
                      )}
                    </div>

                    {isRowDetailOpen && (inputSummary || outputSummary || displayBrief || row.summaryZh) && (
                      <div
                        data-testid="agent-tool-row-detail-drawer"
                        className="mt-1.5 rounded-md border border-sp-line bg-sp-card/70 px-2.5 py-2 space-y-1 text-xs leading-relaxed text-sp-text-secondary break-words whitespace-normal select-text"
                      >
                        {inputSummary && (
                          <div>
                            <span className="font-semibold text-sp-text-muted mr-1.5">
                              传入参数：
                            </span>
                            <code className="font-mono break-all text-sp-text">
                              {inputSummary}
                            </code>
                          </div>
                        )}
                        {(outputSummary || displayBrief) && (
                          <div>
                            <span className="font-semibold text-sp-text-muted mr-1.5">
                              返回结果：
                            </span>
                            <span className="text-sp-text">
                              {outputSummary || displayBrief}
                            </span>
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
