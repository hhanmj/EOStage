/**
 * CatalogResultRow.tsx
 * 切片 12 / W5-3b 检索结果行共用组件（供 ManualWorkbench 与 Agent 右侧面板「结果清单」页签共同复用）
 * 规范：W4 切片 12 + W5-3b 对齐样稿
 */

import React, { useState, useRef, useEffect } from "react";
import { createPortal } from "react-dom";
import {
  Check,
  Pause,
  Play,
  FolderOpen,
  Download,
  Eye,
  Info,
  Loader2,
  ChevronDown,
  ChevronUp,
  Scissors,
} from "lucide-react";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import {
  buildSceneRowMetaDisplay,
  buildSceneSummaryRows,
  buildSceneFullCopyText,
  formatSceneRowDisplayTitle,
} from "@/lib/catalogProductInfo";
import { canSceneShowMapPreview } from "@/hooks/useWorkbenchMapPreview";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { SceneRow } from "@/lib/bridge";
import type { RowTaskState, RowAction, ApplicationTask } from "@/shell/api/types";
import {
  deriveVisibleRowActions,
  buildResultRowBadges,
  formatBytes,
  opticalTileLabel,
} from "@/shell/api/rowProjection";
import { detectNisarSceneReleaseTier } from "@/shell/api/agentSessionWorkspace";
import {
  isRowSelectable,
  failureFirstSentence,
  failureSummaryText,
  rowProgressPercent,
  stripInternalDiagnosticSuffix,
} from "@/shell/api/resultListLogic";
import { pauseReasonLine } from "@/shell/api/taskCardLogic";
import { describeDownloadFailure, formatErrorWithCode } from "@/shell/api/errorExplanations";
import { ThumbnailSlot } from "./ThumbnailSlot";
import { MAP_ASC_COLOR, MAP_DESC_COLOR } from "@/components/WorkbenchMap";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";

export interface CatalogResultRowProps {
  scene: SceneRow;
  batchId?: string | null;
  rowState?: RowTaskState;
  task?: ApplicationTask;
  isSelected?: boolean;
  isChecked?: boolean;
  isCreating?: boolean;
  isFailureExpanded?: boolean;
  family?: string | null;
  hideDownload?: boolean;
  onToggleFailureExpand?: (id: string) => void;
  onSelectScene?: (id: string) => void;
  onHoverChange?: (id: string | null) => void;
  onToggleCheckScene?: (id: string) => void;
  onRowAction?: (id: string, action: RowAction) => void;
  companionStatusText?: string;
  canOpticalAoiClip?: boolean;
  onOpticalAoiClip?: (id: string) => void;
  onPreviewScene?: (scene: SceneRow) => void;
  isPreviewPinned?: boolean;
  onTogglePinPreview?: (scene: SceneRow) => void;
  className?: string;
  style?: React.CSSProperties;
}

export function ProductName({ value }: { value: string }) {
  const separator = value.includes("_") ? "_" : value.includes(".") ? "." : null;
  if (!separator) {
    return <>{value}</>;
  }
  const parts = value.split(separator);
  return (
    <>
      {parts.map((part, index) => (
        <React.Fragment key={`${index}:${part}`}>
          {index > 0 && (
            <>
              {separator}
              <wbr />
            </>
          )}
          {part}
        </React.Fragment>
      ))}
    </>
  );
}

export function CatalogResultRow({
  scene,
  batchId,
  rowState,
  task,
  isSelected = false,
  isChecked = false,
  isCreating = false,
  isFailureExpanded = false,
  family,
  hideDownload = false,
  onToggleFailureExpand,
  onSelectScene,
  onHoverChange,
  onToggleCheckScene,
  onRowAction,
  companionStatusText,
  canOpticalAoiClip = false,
  onOpticalAoiClip,
  onPreviewScene,
  isPreviewPinned = false,
  onTogglePinPreview,
  className,
  style,
}: CatalogResultRowProps) {
  const { t, locale } = useI18n();
  const [infoOpen, setInfoOpen] = useState(false);
  const [infoCopied, setInfoCopied] = useState(false);
  const [infoRect, setInfoRect] = useState<{ left: number; top: number } | null>(null);
  const infoBtnRef = useRef<HTMLButtonElement | null>(null);
  const copyTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current !== null) {
        window.clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  const updateInfoPosition = () => {
    const el = infoBtnRef.current;
    if (!el || typeof window === "undefined") {
      setInfoRect(null);
      return;
    }
    const rect = el.getBoundingClientRect();
    const popW = 408;
    const popH = Math.min(340, Math.max(220, buildSceneSummaryRows(scene).length * 26 + 56));
    const left = Math.max(12, Math.min(rect.left - popW + rect.width, window.innerWidth - popW - 12));
    const top =
      rect.bottom + 6 + popH <= window.innerHeight - 12
        ? rect.bottom + 6
        : Math.max(12, rect.top - popH - 6);
    setInfoRect({ left, top });
  };

  const handleOpenInfo = () => {
    updateInfoPosition();
    setInfoOpen(true);
  };

  const handleCloseInfo = () => {
    setInfoOpen(false);
  };

  const handleCopyInfo = async (e: React.MouseEvent) => {
    e.stopPropagation();
    const fullText = buildSceneFullCopyText(scene);
    try {
      if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
        await navigator.clipboard.writeText(fullText);
      } else {
        const ta = document.createElement("textarea");
        ta.value = fullText;
        ta.style.position = "fixed";
        ta.style.opacity = "0";
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
      }
      setInfoCopied(true);
      if (copyTimerRef.current !== null) window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setInfoCopied(false), 1200);
      emitTopToast({ text: t("mapWorkbench.results.copiedSceneInfo"), tone: "success" });
    } catch {
      emitTopToast({ text: t("mapWorkbench.results.copySceneInfoFailed"), tone: "danger" });
    }
  };

  const summaryRows = infoOpen ? buildSceneSummaryRows(scene) : [];
  const showCreatingSpinner = useDelayedFlag(isCreating);
  const selectable = isRowSelectable(rowState);
  const rawActions = deriveVisibleRowActions(rowState);
  const actions = hideDownload ? rawActions.filter((a) => a !== "download") : rawActions;
  const taskStatus = rowState?.task_status;
  const metaDisplay = buildSceneRowMetaDisplay(scene, family);
  const badges = buildResultRowBadges(scene, metaDisplay.category);
  const displayTitle = formatSceneRowDisplayTitle(scene);
  const acqDate = metaDisplay.compactTime;
  const isOptical = metaDisplay.category === "optical";
  const opticalBadges = isOptical
    ? badges.filter((badge) => !["date", "platform"].includes(badge.key))
    : [];
  const opticalTile = isOptical ? opticalTileLabel(scene) : null;
  const progress = rowProgressPercent(task);
  const diskLowReason = pauseReasonLine(task?.pause_reason);

  // 失败原因展开判定：当完整 message_zh 超过首句、包含可操作建议或诊断码时可展开
  const fullFailureMsg = stripInternalDiagnosticSuffix(task?.failure?.message_zh);
  const firstSentence = failureFirstSentence(fullFailureMsg);
  const hasFailureCodes = Array.isArray(task?.failure?.codes) && task.failure.codes.length > 0;
  const failureDetail =
    taskStatus === "failed"
      ? describeDownloadFailure({ codes: task?.failure?.codes, messageZh: fullFailureMsg }, locale)
      : null;
  const canExpandFailure = Boolean(
    (fullFailureMsg && fullFailureMsg.trim() !== firstSentence.trim()) ||
      failureDetail?.guidance ||
      (failureDetail?.extraExplanations.length ?? 0) > 0 ||
      hasFailureCodes
  );

  return (
    <div
      tabIndex={0}
      data-scene-id={scene.scene_id}
      title={displayTitle}
      onClick={() => onSelectScene?.(scene.scene_id)}
      onMouseEnter={() => onHoverChange?.(scene.scene_id)}
      onMouseLeave={() => onHoverChange?.(null)}
      onFocus={() => onHoverChange?.(scene.scene_id)}
      onBlur={() => onHoverChange?.(null)}
      style={style}
      className={cn(
        "sp-result-row group shrink-0 w-full border-b border-sp-divider px-3 py-2.5 transition-colors duration-fast ease-sp cursor-pointer select-text outline-none focus:outline-none focus-visible:outline-none",
        isSelected
          ? "is-selected bg-sp-selected text-sp-text"
          : "bg-sp-surface hover:bg-sp-hover text-sp-text",
        className
      )}
    >
      <span
        aria-hidden="true"
        data-testid="result-row-cursor"
        className="sp-result-row-cursor"
      />
      <div className="flex items-center gap-3">
        {/* 复选框 */}
        <div className="[grid-area:cb] self-center flex items-center justify-center shrink-0">
          <button
            type="button"
            disabled={!selectable}
            aria-disabled={!selectable}
            aria-label={!selectable ? t("mapWorkbench.row.alreadyInTask") : isChecked ? t("mapWorkbench.row.uncheck") : t("mapWorkbench.row.check")}
            title={!selectable ? t("mapWorkbench.row.alreadyInTask") : isChecked ? t("mapWorkbench.row.uncheck") : t("mapWorkbench.row.check")}
            onClick={(e) => {
              e.stopPropagation();
              if (selectable) {
                onToggleCheckScene?.(scene.scene_id);
              }
            }}
            className={cn(
              "w-5 h-5 rounded-sp-sm flex items-center justify-center transition-colors duration-fast ease-sp shrink-0 cursor-pointer",
              !selectable
                ? "bg-sp-surface-muted ring-1 ring-sp-line cursor-not-allowed"
                : isChecked
                ? "bg-sp-accent-strong text-white"
                : "bg-sp-surface text-transparent ring-1 ring-sp-line hover:ring-sp-accent"
            )}
          >
            {isChecked && <Check className="w-3.5 h-3.5 stroke-[2.5] sp-check-morph-in" />}
          </button>
        </div>

        {/* 缩略图插槽 */}
        <div className="self-center shrink-0">
          <ThumbnailSlot sceneId={scene.scene_id} scene={scene} batchId={batchId} />
        </div>

        {/* 主栏：完整名称、已有科学字段与操作 */}
        <div className="min-w-0 flex-1 flex flex-col justify-center gap-1">
          <div className="flex flex-wrap items-start justify-between gap-x-2 gap-y-1.5 min-w-0">
            <div
              data-testid="product-name"
              className="min-w-[180px] flex-1 font-mono text-xs leading-5 break-words [overflow-wrap:anywhere] text-sp-text"
              title={displayTitle}
            >
              <ProductName value={displayTitle} />
            </div>

            {/* 右侧状态徽章与操作按钮 */}
            <div
              className="flex flex-wrap items-center justify-end gap-1.5 shrink-0 ml-auto max-w-full"
              onClick={(e) => e.stopPropagation()}
            >
              {taskStatus === "queued" && (
                <span className="inline-flex items-center gap-1.5 text-xs text-sp-text-secondary font-medium tabular-nums whitespace-nowrap">
                  <span className="w-2 h-2 rounded-full bg-sp-text-muted" />
                  <span>{t("mapWorkbench.row.queued")}</span>
                </span>
              )}
              {taskStatus === "running" && (
                <span className="inline-flex items-center gap-1.5 text-xs text-sp-accent-strong font-medium tabular-nums whitespace-nowrap">
                  <span className="w-2 h-2 rounded-full bg-sp-accent" />
                  <span>{t("mapWorkbench.row.transferring")}{progress !== null ? ` ${progress}%` : ""}</span>
                </span>
              )}
              {taskStatus === "paused" && (
                <span
                  className="inline-flex items-center gap-1.5 text-xs text-sp-warning-text font-medium tabular-nums whitespace-nowrap"
                  title={diskLowReason ?? undefined}
                >
                  <span className="w-2 h-2 rounded-full bg-sp-status-warning" />
                  <span>{t("mapWorkbench.row.paused")}{progress !== null ? ` ${progress}%` : ""}</span>
                  {diskLowReason && (
                    <span className="text-xs font-normal">{t("mapWorkbench.row.diskLow")}</span>
                  )}
                </span>
              )}
              {taskStatus === "completed" && (
                <span className="inline-flex items-center gap-1.5 text-xs text-sp-success-text font-medium tabular-nums whitespace-nowrap">
                  <span className="w-2 h-2 rounded-full bg-sp-status-success" />
                  <span>{t("mapWorkbench.row.completed")}</span>
                </span>
              )}
              {taskStatus === "failed" && (
                <span
                  className="inline-flex items-center gap-1.5 text-xs text-sp-danger-text font-medium tabular-nums whitespace-nowrap"
                  title={failureSummaryText(task?.failure)}
                >
                  <span className="w-2 h-2 rounded-full bg-sp-status-danger" />
                  <span>{t("mapWorkbench.row.failed")}</span>
                </span>
              )}
              {taskStatus === "cancelled" && (
                <span className="inline-flex items-center gap-1.5 text-xs text-sp-text-secondary font-medium tabular-nums whitespace-nowrap">
                  <span className="w-2 h-2 rounded-full bg-sp-text-muted" />
                  <span>{t("mapWorkbench.row.cancelled")}</span>
                </span>
              )}
              {taskStatus === "interrupted" && (
                <span className="inline-flex items-center gap-1.5 text-xs text-sp-text-secondary font-medium tabular-nums whitespace-nowrap">
                  <span className="w-2 h-2 rounded-full bg-sp-text-muted" />
                  <span>{t("mapWorkbench.row.interrupted")}</span>
                </span>
              )}

              {/* 行内动作按钮 */}
              {onPreviewScene &&
                (scene.scene_id.startsWith("gee-composite::") ||
                  scene.scene_id.startsWith("gee-scene::")) && (
                  <button
                    type="button"
                    data-testid="row-gee-preview"
                    aria-pressed={isPreviewPinned}
                    onClick={(e) => {
                      e.stopPropagation();
                      onPreviewScene(scene);
                    }}
                    aria-label={t("mapWorkbench.results.previewGeeLayerAria")}
                    title={t("mapWorkbench.results.previewGeeLayerAria")}
                    className={cn(
                      "sp-row-action-btn h-7 px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
                      isPreviewPinned
                        ? "bg-sp-accent/15 text-sp-accent font-semibold ring-1 ring-sp-accent/40"
                        : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted",
                    )}
                  >
                    <Eye className="w-3.5 h-3.5" />
                    <span>{t("mapWorkbench.results.previewAction")}</span>
                  </button>
                )}

              {onTogglePinPreview &&
                (isPreviewPinned || canSceneShowMapPreview(scene, batchId)) && (
                  <button
                    type="button"
                    data-testid="row-preview-toggle"
                    aria-pressed={isPreviewPinned}
                    onClick={(e) => {
                      e.stopPropagation();
                      onTogglePinPreview(scene);
                    }}
                    title={isPreviewPinned ? t("mapWorkbench.results.unpinMapPreviewTitle") : t("mapWorkbench.results.pinMapPreviewTitle")}
                    className={cn(
                      "sp-row-action-btn h-7 px-1.5 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
                      isPreviewPinned
                        ? "bg-sp-accent/15 text-sp-accent font-semibold ring-1 ring-sp-accent/40"
                        : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted",
                    )}
                  >
                    <Eye className="w-3 h-3" />
                    <span>{t("mapWorkbench.results.previewAction")}</span>
                  </button>
                )}

              <button
                ref={infoBtnRef}
                type="button"
                data-testid="row-info-copy"
                aria-label={t("mapWorkbench.results.inspectCopyMetadataAria")}
                title={t("mapWorkbench.results.inspectCopyMetadataTitle")}
                onMouseEnter={handleOpenInfo}
                onMouseLeave={handleCloseInfo}
                onFocus={handleOpenInfo}
                onBlur={handleCloseInfo}
                onClick={(e) => {
                  void handleCopyInfo(e);
                }}
                className={cn(
                  "sp-row-action-btn h-7 w-7 inline-flex items-center justify-center rounded-sp-sm text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
                  infoCopied
                    ? "bg-sp-success-bg text-sp-success-text"
                    : "text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted",
                )}
              >
                {infoCopied ? <Check className="w-3.5 h-3.5" /> : <Info className="w-3.5 h-3.5" />}
              </button>

              {infoOpen &&
                summaryRows.length > 0 &&
                typeof document !== "undefined" &&
                createPortal(
                  <div
                    role="tooltip"
                    data-testid="row-info-popover"
                    style={
                      infoRect
                        ? { position: "fixed", left: infoRect.left, top: infoRect.top, width: 408 }
                        : { position: "fixed", left: 16, top: 16, width: 408 }
                    }
                    className="pointer-events-none z-[9999] rounded-sp-md border border-sp-line bg-sp-surface/98 p-2.5 shadow-overlay text-xs text-sp-text backdrop-blur-md"
                  >
                    <div className="flex flex-col gap-1">
                      {summaryRows.map((entry) => (
                        <div key={entry.label} className="grid grid-cols-[minmax(88px,auto)_1fr] items-start gap-2.5">
                          <span className="text-xs text-sp-text-muted whitespace-nowrap shrink-0">{entry.label}</span>
                          <span className="text-xs font-mono text-sp-text break-all leading-4">
                            {entry.value}
                          </span>
                        </div>
                      ))}
                    </div>
                    <div className="mt-1.5 pt-1.5 border-t border-sp-line text-xs text-sp-text-muted">
                      {t("mapWorkbench.results.clickInfoToCopyHint")}
                    </div>
                  </div>,
                  document.body,
                )}

              {isOptical && canOpticalAoiClip && onOpticalAoiClip && (
                <button
                  type="button"
                  data-testid="btn-optical-aoi-clip-row"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpticalAoiClip(scene.scene_id);
                  }}
                  aria-label={t("panels.opticalAoiClip.actionRow")}
                  title={t("panels.opticalAoiClip.actionRowTitle")}
                  className="sp-row-action-btn h-7 px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <Scissors className="w-3.5 h-3.5" />
                  <span>{t("panels.opticalAoiClip.actionRow")}</span>
                </button>
              )}

              {actions.includes("download") && (
                <button
                  type="button"
                  onClick={() => onRowAction?.(scene.scene_id, "download")}
                  disabled={isCreating}
                  aria-busy={isCreating}
                  aria-label={t("mapWorkbench.row.download")}
                  title={isCreating ? t("mapWorkbench.results.confirmingAccount") : t("mapWorkbench.results.addToQueue")}
                  className="sp-row-action-btn h-7 min-w-[68px] justify-center px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50"
                >
                  <span
                    data-testid="row-download-icon-morph"
                    data-state={isCreating ? "creating" : "idle"}
                    className="sp-action-icon-morph"
                  >
                    <Download
                      className={cn(
                        "w-3.5 h-3.5",
                        showCreatingSpinner ? "is-hidden" : "is-shown"
                      )}
                    />
                    <Loader2
                      className={cn(
                        "w-3.5 h-3.5 animate-spin text-sp-accent",
                        showCreatingSpinner ? "is-shown" : "is-hidden"
                      )}
                    />
                  </span>
                  <span>{t("mapWorkbench.row.download")}</span>
                </button>
              )}

              {actions.includes("pause") && (
                <button
                  type="button"
                  onClick={() => onRowAction?.(scene.scene_id, "pause")}
                  aria-label={t("mapWorkbench.row.pause")}
                  title={t("mapWorkbench.row.pauseDownload")}
                  className="sp-row-action-btn h-7 px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <Pause className="w-3.5 h-3.5" />
                  <span>{t("mapWorkbench.row.pause")}</span>
                </button>
              )}

              {actions.includes("resume") && (
                <button
                  type="button"
                  onClick={() => onRowAction?.(scene.scene_id, "resume")}
                  aria-label={t("mapWorkbench.row.resume")}
                  title={t("mapWorkbench.row.resumeDownload")}
                  className="sp-row-action-btn h-7 px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <Play className="w-3.5 h-3.5" />
                  <span>{t("mapWorkbench.row.resume")}</span>
                </button>
              )}

              {actions.includes("open_folder") && (
                <button
                  type="button"
                  onClick={() => onRowAction?.(scene.scene_id, "open_folder")}
                  aria-label={t("mapWorkbench.row.openDirectory")}
                  title={t("mapWorkbench.row.openDirectoryTitle")}
                  className="sp-row-action-btn h-7 px-2 inline-flex items-center gap-1 rounded-sp-sm text-xs font-medium text-sp-text-secondary hover:text-sp-text hover:bg-sp-surface-muted transition-colors duration-fast ease-sp cursor-pointer"
                >
                  <FolderOpen className="w-3.5 h-3.5" />
                  <span>{t("mapWorkbench.row.directory")}</span>
                </button>
              )}
            </div>
          </div>

          {/* Line 3: companion status badge */}
          {companionStatusText && (
            <div data-testid="row-companion-badge" className="flex items-center gap-1.5 text-xs text-sp-accent mt-0.5">
              <span className="w-1.5 h-1.5 rounded-full bg-sp-accent shrink-0" />
              <span className="font-medium">{companionStatusText}</span>
            </div>
          )}

          {/* Line 2: compact category-aware scientific metadata */}
          {(() => {
            const line2Items: React.ReactNode[] = [];
            if (acqDate) {
              line2Items.push(<span key="date" className="tabular-nums">{acqDate}</span>);
            }
            if (metaDisplay.category === "optical") {
              if (metaDisplay.platform) line2Items.push(<span key="plat">{metaDisplay.platform}</span>);
              if (opticalTile) line2Items.push(<span key="tile" className="font-mono">{opticalTile}</span>);
              for (const badge of opticalBadges) {
                line2Items.push(
                  <span key={badge.key} className={badge.key === "size" ? "font-mono tabular-nums" : undefined}>
                    {badge.value}
                  </span>
                );
              }
            } else if (metaDisplay.category === "sar") {
              if (metaDisplay.orbitDotKind) {
                const isAsc = metaDisplay.orbitDotKind === "asc";
                line2Items.push(
                  <span key="dir" className="inline-flex items-center gap-1">
                    <span
                      className="w-1.5 h-1.5 rounded-full shrink-0"
                      style={{ backgroundColor: isAsc ? MAP_ASC_COLOR : MAP_DESC_COLOR }}
                    />
                    <span>{isAsc ? t("mapWorkbench.row.ascending") : t("mapWorkbench.row.descending")}</span>
                  </span>
                );
              }
              if (metaDisplay.orbitTrackText) {
                const sp = metaDisplay.orbitTrackText.indexOf(" ");
                const labelPart = sp > 0 ? metaDisplay.orbitTrackText.slice(0, sp) : "Track";
                const numPart = sp > 0 ? metaDisplay.orbitTrackText.slice(sp + 1) : metaDisplay.orbitTrackText;
                line2Items.push(<span key="track">{labelPart} <span className="font-mono">{numPart}</span></span>);
              }
              if (metaDisplay.frameText) {
                const sp = metaDisplay.frameText.indexOf(" ");
                const numPart = sp > 0 ? metaDisplay.frameText.slice(sp + 1) : metaDisplay.frameText;
                line2Items.push(<span key="frame">Frame <span className="font-mono">{numPart}</span></span>);
              }
              if (metaDisplay.polarization) {
                line2Items.push(<span key="pol">{metaDisplay.polarization}</span>);
              }
              if (metaDisplay.levelOrType) {
                line2Items.push(<span key="level">{metaDisplay.levelOrType}</span>);
              }
              if (metaDisplay.sizeLabel) {
                line2Items.push(<span key="size" className="font-mono tabular-nums">{metaDisplay.sizeLabel}</span>);
              }
              const tier = detectNisarSceneReleaseTier(scene) ?? metaDisplay.tier;
              if (tier) {
                line2Items.push(
                  <span
                    key="tier"
                    data-testid="row-nisar-tier-badge"
                    title={
                      tier === "validated"
                        ? t("mapWorkbench.results.nisarValidatedTitle")
                        : t("mapWorkbench.results.nisarProvisionalTitle")
                    }
                    className={cn(
                      "inline-flex items-center px-1.5 py-0 rounded text-xs font-medium whitespace-nowrap",
                      tier === "validated"
                        ? "bg-sp-success-bg text-sp-success-text"
                        : "bg-sp-warning-bg text-sp-warning-text"
                    )}
                  >
                    {tier === "validated" ? "Validated" : "Provisional"}
                  </span>
                );
              }
            } else {
              if (metaDisplay.platform) line2Items.push(<span key="plat">{metaDisplay.platform}</span>);
              if (metaDisplay.placeOrStation) line2Items.push(<span key="loc">{metaDisplay.placeOrStation}</span>);
              if (metaDisplay.levelOrType) line2Items.push(<span key="type">{metaDisplay.levelOrType}</span>);
              if (metaDisplay.polarization) line2Items.push(<span key="pol">{metaDisplay.polarization}</span>);
              if (metaDisplay.scaleOrSpanText) line2Items.push(<span key="span">{metaDisplay.scaleOrSpanText}</span>);
              if (metaDisplay.cloudText) line2Items.push(<span key="cloud">{metaDisplay.cloudText}</span>);
              if (metaDisplay.sizeLabel) line2Items.push(<span key="size" className="font-mono tabular-nums">{metaDisplay.sizeLabel}</span>);
            }
            return (
              <div className="flex flex-wrap items-center gap-x-1.5 text-xs leading-5 text-sp-text-muted">
                {line2Items.map((item, idx) => (
                  <React.Fragment key={idx}>
                    {idx > 0 && <span>·</span>}
                    {item}
                  </React.Fragment>
                ))}
              </div>
            );
          })()}

          {/* 失败首句及展开切换 */}
          {taskStatus === "failed" && (
            <div className="flex flex-wrap items-center text-xs text-sp-danger-text break-all mt-1">
              <span title={failureSummaryText(task?.failure)}>
                {failureSummaryText(task?.failure)}
              </span>
              {canExpandFailure && (
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    onToggleFailureExpand?.(scene.scene_id);
                  }}
                  aria-expanded={isFailureExpanded}
                  className="ml-2 inline-flex items-center gap-0.5 text-xs text-sp-accent-strong hover:underline cursor-pointer"
                >
                  <span>{isFailureExpanded ? t("mapWorkbench.row.collapse") : t("mapWorkbench.row.expandReason")}</span>
                  {isFailureExpanded ? (
                    <ChevronUp className="w-3.5 h-3.5" />
                  ) : (
                    <ChevronDown className="w-3.5 h-3.5" />
                  )}
                </button>
              )}
            </div>
          )}

          {/* 失败原因完整展开区域（真实失败原因 + 建议操作优先，内部错误码仅作弱化诊断参考） */}
          {taskStatus === "failed" && isFailureExpanded && failureDetail && (
            <div
              data-testid="row-failure-detail"
              className="mt-2 p-2.5 rounded-sp-sm bg-sp-card border border-sp-line text-xs text-sp-text-secondary max-h-[240px] overflow-y-auto select-text space-y-1.5"
              onClick={(e) => e.stopPropagation()}
            >
              {failureDetail.cause && (
                <div
                  data-testid="row-failure-cause"
                  className="whitespace-pre-wrap break-words leading-relaxed text-sp-text"
                >
                  {failureDetail.cause}
                </div>
              )}
              {failureDetail.extraExplanations.length > 0 && (
                <div className="space-y-0.5 text-sp-text-secondary leading-relaxed">
                  {failureDetail.extraExplanations.map((text) => (
                    <div key={text}>{text}</div>
                  ))}
                </div>
              )}
              {failureDetail.guidance && (
                <div
                  data-testid="row-failure-guidance"
                  className="rounded-sp-sm bg-sp-surface-muted/70 px-2 py-1.5 text-xs text-sp-text-secondary leading-relaxed"
                >
                  <span className="font-medium text-sp-text">
                    {t("mapWorkbench.row.failureGuidanceLabel")}：
                  </span>
                  <span>{failureDetail.guidance}</span>
                </div>
              )}
              {failureDetail.diagnosticCodes.length > 0 && (
                <div
                  data-testid="row-failure-diagnostic-ref"
                  className="pt-1 border-t border-sp-divider text-xs text-sp-text-muted/80 font-mono"
                  title={failureDetail.diagnosticCodes
                    .map((code) => formatErrorWithCode(code))
                    .join(" · ")}
                >
                  {t("mapWorkbench.row.diagnosticRef", {
                    codes: failureDetail.diagnosticCodes.join(", "),
                  })}
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
