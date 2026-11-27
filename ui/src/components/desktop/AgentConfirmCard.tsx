/**
 * AgentConfirmCard.tsx
 * Agent 确认卡独立组件（支持自动执行、超额回落与逐项确认三种呈现模式）
 * 规范：v3-contracts.ts、briefs/w5-2f-permission-ui.md、A1 §3.1.1
 */

import React, { useState, useId, useMemo } from "react";
import { ShieldCheck, MapPin, Download, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type {
  StreamConfirmCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import type { AgentCardSnapshot } from "../../lib/generated/v3-contracts";
import type { ApplicationTask } from "../../shell/api/types";
import {
  projectConfirmCard,
  buildConfirmCardTaskModels,
} from "../../shell/api/agentConversationLogic";
import { AgentTaskCard } from "./AgentTaskCard";
import { projectApproval } from "../../shell/api/approvalLogic";
import { truncateMiddle } from "../../shell/api/rowProjection";
import {
  projectOutputDirRow,
  approveOutputDirArg,
  defaultDirOffer,
  validateLocalOutputDir,
} from "../../shell/api/confirmCardOutputDir";
import { resolveApprovedOutputDir } from "../../shell/api/approvedOutputDirMemory";
import { mappedSourceName } from "../../shell/api/sourceDisplayName";
import { formatByteSize } from "@/lib/formatByteSize";
import { MAP_ASC_COLOR, MAP_DESC_COLOR } from "../WorkbenchMap";
import { useI18n } from "@/i18n";
import {
  isSentinel1OrbitApplicable,
  getPreciseOrbitPreference,
  setPreciseOrbitPreference,
} from "@/lib/preciseOrbitPreferences";

interface ConfirmManifestItem {
  id: string;
  title: string;
  date?: string | null;
  orbit?: string | null;
  cloudPct?: number | null;
  sizeBytes?: number | null;
}

function asNonEmptyString(val: unknown): string | null {
  return typeof val === "string" && val.trim().length > 0 ? val.trim() : null;
}

function extractSnapshotFilterBadges(snapshot: Record<string, unknown>): string[] {
  const badges: string[] = [];
  const rawFilters =
    (snapshot.query_filters && typeof snapshot.query_filters === "object"
      ? (snapshot.query_filters as Record<string, unknown>)
      : null) ??
    (snapshot.filters && typeof snapshot.filters === "object"
      ? (snapshot.filters as Record<string, unknown>)
      : null) ??
    (snapshot.range &&
    typeof snapshot.range === "object" &&
    (snapshot.range as Record<string, unknown>).filters &&
    typeof (snapshot.range as Record<string, unknown>).filters === "object"
      ? ((snapshot.range as Record<string, unknown>).filters as Record<string, unknown>)
      : null);
  if (!rawFilters) return badges;

  const cloud =
    typeof rawFilters.cloud_cover_max === "number"
      ? rawFilters.cloud_cover_max
      : typeof rawFilters.max_cloud_cover === "number"
        ? rawFilters.max_cloud_cover
        : typeof rawFilters.cloud_max === "number"
          ? rawFilters.cloud_max
          : null;
  if (cloud !== null && Number.isFinite(cloud)) {
    badges.push(`云量 ≤ ${cloud}%`);
  }

  const productType = asNonEmptyString(rawFilters.product_type);
  if (productType) badges.push(`产品 ${productType}`);

  const platformList = Array.isArray(rawFilters.platforms)
    ? rawFilters.platforms.map(asNonEmptyString).filter(Boolean)
    : asNonEmptyString(rawFilters.platform)
      ? [asNonEmptyString(rawFilters.platform)!]
      : [];
  if (platformList.length > 0) badges.push(`平台 ${platformList.join("/")}`);

  const pols = Array.isArray(rawFilters.polarizations)
    ? rawFilters.polarizations.map(asNonEmptyString).filter(Boolean)
    : asNonEmptyString(rawFilters.polarization)
      ? [asNonEmptyString(rawFilters.polarization)!]
      : [];
  if (pols.length > 0) badges.push(`极化 ${pols.join("+")}`);

  const beams = Array.isArray(rawFilters.beam_modes)
    ? rawFilters.beam_modes.map(asNonEmptyString).filter(Boolean)
    : asNonEmptyString(rawFilters.beam_mode)
      ? [asNonEmptyString(rawFilters.beam_mode)!]
      : [];
  if (beams.length > 0) badges.push(`模式 ${beams.join("/")}`);

  const freq = asNonEmptyString(rawFilters.frequency);
  if (freq) badges.push(`频段 ${freq}`);

  const relOrbit = rawFilters.relative_orbit;
  if (
    (typeof relOrbit === "number" && Number.isFinite(relOrbit)) ||
    (typeof relOrbit === "string" && relOrbit.trim().length > 0)
  ) {
    badges.push(`相对轨道 ${relOrbit}`);
  }

  const path = rawFilters.path ?? rawFilters.wrs_path;
  const rowVal = rawFilters.row ?? rawFilters.wrs_row;
  if (path !== undefined && path !== null && String(path).trim()) {
    badges.push(
      rowVal !== undefined && rowVal !== null && String(rowVal).trim()
        ? `条带 ${path}/${rowVal}`
        : `条带 Path ${path}`
    );
  }
  const frame = rawFilters.frame;
  if (frame !== undefined && frame !== null && String(frame).trim()) {
    badges.push(`Frame ${frame}`);
  }
  return badges;
}

function extractManifestItems(snapshot: Record<string, unknown>): ConfirmManifestItem[] {
  const rawList = Array.isArray(snapshot.selected_items_preview)
    ? snapshot.selected_items_preview
    : Array.isArray(snapshot.selected_items)
      ? snapshot.selected_items
      : [];
  const items: ConfirmManifestItem[] = [];
  for (let i = 0; i < rawList.length; i++) {
    const entry = rawList[i];
    if (typeof entry === "string" && entry.trim().length > 0) {
      items.push({ id: entry.trim(), title: entry.trim() });
      continue;
    }
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) continue;
    const rec = entry as Record<string, unknown>;
    const id =
      asNonEmptyString(rec.item_id) ??
      asNonEmptyString(rec.scene_id) ??
      asNonEmptyString(rec.id) ??
      `item-${i + 1}`;
    const title =
      asNonEmptyString(rec.title) ??
      asNonEmptyString(rec.scene_id) ??
      asNonEmptyString(rec.item_id) ??
      id;
    const date =
      asNonEmptyString(rec.acquisition_date) ??
      asNonEmptyString(rec.acquisition_time) ??
      asNonEmptyString(rec.datetime) ??
      asNonEmptyString(rec.date);
    const orbit =
      asNonEmptyString(rec.orbit_direction) ??
      (typeof rec.relative_orbit === "number" ? `Track ${rec.relative_orbit}` : undefined);
    const cloudPct =
      typeof rec.cloud_cover === "number" && Number.isFinite(rec.cloud_cover)
        ? rec.cloud_cover
        : typeof rec.cloud_pct === "number" && Number.isFinite(rec.cloud_pct)
          ? rec.cloud_pct
          : null;
    const sizeBytes =
      typeof rec.size_bytes === "number" && Number.isFinite(rec.size_bytes) && rec.size_bytes > 0
        ? rec.size_bytes
        : typeof rec.estimated_bytes === "number" && Number.isFinite(rec.estimated_bytes) && rec.estimated_bytes > 0
          ? rec.estimated_bytes
          : null;
    items.push({ id, title, date, orbit, cloudPct, sizeBytes });
  }
  return items;
}

export interface AgentConfirmCardProps {
  item: StreamConfirmCardItem;
  sourceNames?: Readonly<Record<string, string>>;
  /** 来源显示名（同 run 计划卡披露）；快照 source 是 provider id，不显示 */
  sourceLabel?: string;
  /** 同一 run 检索条件补全徽章（例如云量/传感器/平台等） */
  extraConditionBadges?: string[];
  /** 批准时实际使用的下载目录（卡上所选，见 approvedOutputDirMemory） */
  approvedOutputDir?: string | null;
  subsequentProgress?: StreamProgressCardItem | null;
  createdTaskIds?: string[];
  agentTasks?: ApplicationTask[];
  onTaskAction?: (
    taskId: string,
    revision: number,
    action: "pause" | "resume" | "cancel" | "retry"
  ) => Promise<unknown> | unknown;
  autoCancelOutcome?: string | null;
  defaultDownloadDir?: string;
  onPickDirectory?: () => Promise<string | null>;
  onApprove?: (
    planId: string,
    snapshot: AgentCardSnapshot,
    outputDir?: string,
    includePreciseOrbit?: boolean,
    options?: { nisarSubset?: boolean }
  ) => void;
  onPreciseOrbitChange?: (enabled: boolean) => void;
  onReject?: (planId: string) => void;
  onViewOnMap?: () => void;
  onCancelAuto?: (runId: string, createdTaskIds: string[]) => void;
}

export function AgentConfirmCard({
  item,
  sourceNames,
  sourceLabel,
  extraConditionBadges = [],
  approvedOutputDir = null,
  subsequentProgress = null,
  createdTaskIds = [],
  agentTasks = [],
  onTaskAction,
  autoCancelOutcome = null,
  defaultDownloadDir,
  onPickDirectory,
  onApprove,
  onPreciseOrbitChange,
  onReject,
  onViewOnMap,
  onCancelAuto,
}: AgentConfirmCardProps) {
  const { t } = useI18n();
  const inputId = useId();
  const [chosenDir, setChosenDir] = useState<string | null>(null);
  const [manifestExpanded, setManifestExpanded] = useState(false);
  const snapshotRecord = (item.snapshot ?? {}) as unknown as Record<string, unknown>;
  const snapshotDisplayName = asNonEmptyString(snapshotRecord.source_display_name_zh);
  const snapshotResourceId = asNonEmptyString(snapshotRecord.source_resource_id);
  const sourceIdentifier = item.snapshot?.source || snapshotResourceId || sourceLabel || "";
  const productIdentifier = item.snapshot?.range?.product || "";
  const preciseOrbitApplicable = isSentinel1OrbitApplicable(sourceIdentifier, productIdentifier);
  const [includePreciseOrbit, setIncludePreciseOrbit] = useState<boolean>(() =>
    preciseOrbitApplicable
      ? getPreciseOrbitPreference(sourceIdentifier, productIdentifier)
      : false
  );
  const isNisarProduct = useMemo(() => {
    const hay = `${snapshotResourceId ?? ""} ${sourceIdentifier} ${sourceLabel ?? ""} ${productIdentifier}`.toLowerCase();
    return hay.includes("nisar");
  }, [snapshotResourceId, sourceIdentifier, sourceLabel, productIdentifier]);
  const [includeNisarSubset, setIncludeNisarSubset] = useState<boolean>(
    () => snapshotRecord.nisar_subset_available !== false
  );

  const handlePreciseOrbitToggle = (enabled: boolean) => {
    setIncludePreciseOrbit(enabled);
    setPreciseOrbitPreference(sourceIdentifier, enabled, productIdentifier);
    onPreciseOrbitChange?.(enabled);
  };
  const [dirError, setDirError] = useState<string | null>(null);
  const [editingDir, setEditingDir] = useState<boolean>(false);
  const [dirDraft, setDirDraft] = useState<string>("");

  const proj = projectConfirmCard(item, subsequentProgress);
  const trimmedSourceLabel = sourceLabel?.trim() || "";
  const sourceText =
    (trimmedSourceLabel && trimmedSourceLabel !== t("agent.unknown") ? trimmedSourceLabel : null) ??
    snapshotDisplayName ??
    (snapshotResourceId ? mappedSourceName(snapshotResourceId, sourceNames) : null) ??
    (trimmedSourceLabel || t("agent.unknown"));
  const manifestItems = useMemo(() => extractManifestItems(snapshotRecord), [snapshotRecord]);
  const allConditionBadges = useMemo(() => {
    const combined = [
      ...proj.conditionBadges,
      ...extractSnapshotFilterBadges(snapshotRecord),
      ...extraConditionBadges,
    ];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const b of combined) {
      const clean = b.trim();
      if (!clean || seen.has(clean)) continue;
      seen.add(clean);
      out.push(clean);
    }
    return out;
  }, [proj.conditionBadges, snapshotRecord, extraConditionBadges]);
  const effectiveEstimatedSizeZh = useMemo(() => {
    const hasKnownSnapshotBytes =
      typeof item.snapshot?.estimated_bytes === "number" &&
      Number.isFinite(item.snapshot.estimated_bytes) &&
      item.snapshot.estimated_bytes > 0;
    if (hasKnownSnapshotBytes) {
      return proj.estimatedSizeZh;
    }
    const explicitTotal =
      typeof snapshotRecord.total_bytes === "number" && Number.isFinite(snapshotRecord.total_bytes) && snapshotRecord.total_bytes > 0
        ? snapshotRecord.total_bytes
        : manifestItems.reduce((acc, m) => acc + (m.sizeBytes ?? 0), 0);
    return explicitTotal > 0 ? formatByteSize(explicitTotal) : proj.estimatedSizeZh;
  }, [item.snapshot?.estimated_bytes, proj.estimatedSizeZh, snapshotRecord.total_bytes, manifestItems]);

  const effectiveCreatedTaskIds = Array.from(
    new Set([
      ...(Array.isArray(createdTaskIds) ? createdTaskIds : []),
      ...(Array.isArray(subsequentProgress?.created_task_ids)
        ? subsequentProgress.created_task_ids
        : []),
    ])
  );

  // 已批准卡的目录：批准时记下的目录（进程内存，标「卡上选择」）→ 本卡已建任务的保存目录
  // （任务总览 output_root，真实位置；卡上改过目录后重启时快照仍是旧值）→ 快照目录 →「未知」。
  const approved = proj.state === "approved";
  const recalledDir =
    approved && chosenDir === null && approvedOutputDir && approvedOutputDir.trim().length > 0
      ? approvedOutputDir
      : null;
  const baseRow = projectOutputDirRow(item.snapshot, chosenDir ?? recalledDir);
  const taskDir =
    approved && chosenDir === null && recalledDir === null
      ? resolveApprovedOutputDir(null, effectiveCreatedTaskIds, agentTasks)
      : null;
  // 任务目录与快照目录相同时保留快照的来源徽章（默认目录 / 消息中指定的目录）；不同则来源不明，不标
  const row =
    taskDir !== null
      ? {
          ...baseRow,
          dir: taskDir,
          sourceLabel: taskDir === baseRow.dir ? baseRow.sourceLabel : null,
          approveBlocked: false,
          hint: null,
        }
      : baseRow;
  const defaultOffer = defaultDirOffer(defaultDownloadDir, row.dir);

  // 目录已定（已批准，或在卡上选了目录）时「下载目录未知，请在卡上选择」不再成立
  const approvalForDisplay =
    (approved || row.dir !== null) && item.approval
      ? {
          ...item.approval,
          card_reasons: (item.approval.card_reasons ?? []).filter(
            (reason) => reason !== "output_dir_unknown"
          ),
        }
      : item.approval;
  const approvalProj = projectApproval(approvalForDisplay, item.auto_executed);
  const isAuto = approvalProj.kind === "auto";
  const hasReasons = approvalProj.kind === "reasons";
  const approvedTaskModels =
    proj.state === "approved"
      ? buildConfirmCardTaskModels(effectiveCreatedTaskIds, agentTasks, {
          sourceNames,
          sourceLabel: sourceText,
          estimatedSizeZh: proj.estimatedSizeZh,
          outputDir: proj.outputDir,
        })
      : [];

  const isPendingManual = proj.state === "pending" && !isAuto;

  return (
    <div
      id="agent-confirm-card"
      data-testid="eo-confirm-card"
      className={cn(
        "cds-decision-card",
        isPendingManual ? "eo-concentric-band is-pending-approval" : "eo-concentric-neutral",
        "w-full"
      )}
      aria-label={t("agent.confirm.aria")}
    >
      {/* 同心圆外框眉栏（EOStage ChatArtifactCardEyebrow 风格 + MCP 决策顶栏） */}
      <div className="cds-decision-header flex items-center gap-2 px-3 py-1.5">
        <ShieldCheck className="h-3.5 w-3.5 text-sp-accent shrink-0" />
        <span className="text-xs font-semibold text-sp-text flex-1 tracking-tight">
          {t("agent.confirm.title")}
        </span>

        {/* 状态 / 模式徽章 */}
        {isAuto ? (
          <>
            <span className="rounded-full bg-sp-card text-sp-text-secondary border border-sp-line px-2 py-0.5 text-xs font-medium">
              {t("agent.confirm.autoExecuted")}
            </span>
            {approvalProj.modeLabel && (
              <span className="rounded-full bg-sp-card text-sp-text-secondary border border-sp-line px-2 py-0.5 text-xs font-medium">
                {approvalProj.modeLabel}
              </span>
            )}
          </>
        ) : (
          <>
            {proj.state === "approved" && (
              <span className="rounded-full bg-sp-success-bg text-sp-success-text px-2 py-0.5 text-xs font-semibold">
                {t("agent.confirm.approved")}
              </span>
            )}
            {proj.state === "rejected" && (
              <span className="rounded-full bg-sp-danger-bg text-sp-danger-text px-2 py-0.5 text-xs font-semibold">
                {t("agent.confirm.rejected")}
              </span>
            )}
            {proj.state === "expired" && (
              <span className="rounded-full bg-sp-warning-bg text-sp-warning-text px-2 py-0.5 text-xs font-semibold">
                {t("agent.confirm.expired")}
              </span>
            )}
            {proj.state === "superseded" && (
              <span
                data-testid="agent-confirm-superseded-badge"
                className="rounded-full bg-sp-surface text-sp-text-muted border border-sp-line px-2 py-0.5 text-xs font-medium"
              >
                {t("agent.confirm.superseded")}
              </span>
            )}
          </>
        )}
      </div>

      {/* 同心圆内层纯白卡片 */}
      <div className={cn("eo-concentric-inner p-3.5 flex flex-col gap-3", proj.state === "pending" && !isAuto && "eo-artifact-card-pulse")}>
      {/* 事实条目区 */}
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-xs items-baseline m-0">
        <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.source")}</dt>
        <dd className="m-0 text-sp-text flex flex-wrap gap-1.5 items-center">
          <span>{sourceText}</span>
        </dd>

        {(allConditionBadges.length > 0 || proj.orbitBadges.length > 0) && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.conditions")}</dt>
            <dd className="m-0 text-sp-text flex flex-wrap gap-1.5 items-center">
              {allConditionBadges.map((badge, bIdx) => (
                <span
                  key={bIdx}
                  className="rounded-sp-sm bg-sp-card px-2 py-1 text-xs text-sp-text"
                >
                  {badge}
                </span>
              ))}
              {proj.orbitBadges.map((ob, obIdx) => (
                <span
                  key={`orbit-${obIdx}`}
                  className="inline-flex items-center gap-1 rounded-sp-sm bg-sp-card px-2 py-1 text-xs text-sp-text"
                >
                  {ob.tone === "asc" && (
                    <span
                      className="w-1.5 h-1.5 rounded-full shrink-0"
                      style={{ backgroundColor: MAP_ASC_COLOR }}
                    />
                  )}
                  {ob.tone === "desc" && (
                    <span
                      className="w-1.5 h-1.5 rounded-full shrink-0"
                      style={{ backgroundColor: MAP_DESC_COLOR }}
                    />
                  )}
                  <span>{ob.label}</span>
                </span>
              ))}
            </dd>
          </>
        )}

        {proj.resultLine && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.confirm.resultCount")}</dt>
            <dd className="m-0 text-sp-text font-medium">{proj.resultLine}</dd>
          </>
        )}

        {proj.aoiLine && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">AOI</dt>
            <dd className="m-0 text-sp-text">{proj.aoiLine}</dd>
          </>
        )}

        <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.confirm.estimatedSize")}</dt>
        <dd className="m-0 text-sp-text font-medium">{effectiveEstimatedSizeZh}</dd>

        {isNisarProduct && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">NISAR 裁剪</dt>
            <dd className="m-0 text-sp-text flex flex-col gap-1">
              <label className="inline-flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  data-testid="agent-confirm-nisar-subset"
                  checked={includeNisarSubset}
                  disabled={proj.state !== "pending" || isAuto}
                  onChange={(e) => setIncludeNisarSubset(e.target.checked)}
                  className="rounded border-sp-line"
                />
                <span>按 AOI 范围裁剪 (HDF5 HTTP Range)</span>
              </label>
              <span className="text-xs text-sp-text-muted">
                {includeNisarSubset
                  ? "支持按 AOI 自动子集提取，避免全景数十 GB 下载"
                  : "已切换为完整原始 HDF5 产品下载"}
              </span>
            </dd>
          </>
        )}

        {manifestItems.length > 0 && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">已选清单</dt>
            <dd className="m-0 text-sp-text flex flex-col gap-1.5">
              <button
                type="button"
                data-testid="agent-confirm-manifest-toggle"
                aria-expanded={manifestExpanded}
                onClick={() => setManifestExpanded((prev) => !prev)}
                className="inline-flex items-center gap-1.5 text-xs font-medium text-sp-accent hover:underline cursor-pointer w-fit"
              >
                <ChevronRight
                  className={cn(
                    "h-3.5 w-3.5 transition-transform duration-fast ease-sp",
                    manifestExpanded && "rotate-90"
                  )}
                />
                <span>
                  {manifestExpanded
                    ? `收起已选清单（${manifestItems.length} 景）`
                    : `展开查看已选清单（${manifestItems.length} 景）`}
                </span>
              </button>
              {manifestExpanded && (
                <div
                  data-testid="agent-confirm-manifest-list"
                  className="max-h-40 overflow-y-auto rounded-sp-sm border border-sp-line bg-sp-card divide-y divide-sp-divider"
                >
                  {manifestItems.map((m) => (
                    <div
                      key={m.id}
                      className="px-2.5 py-1.5 flex items-center justify-between gap-2 text-xs"
                    >
                      <span className="font-mono text-sp-text truncate" title={m.title}>
                        {truncateMiddle(m.title, 42)}
                      </span>
                      <span className="shrink-0 text-sp-text-muted tabular-nums">
                        {[
                          m.date ? m.date.slice(0, 10) : null,
                          m.orbit,
                          m.cloudPct !== null && m.cloudPct !== undefined
                            ? `云量 ${Math.round(m.cloudPct)}%`
                            : null,
                          m.sizeBytes ? formatByteSize(m.sizeBytes) : null,
                        ]
                          .filter(Boolean)
                          .join(" · ") || "已选中"}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </dd>
          </>
        )}

        {preciseOrbitApplicable && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.confirm.preciseOrbit")}</dt>
            <dd className="m-0 text-sp-text flex flex-col gap-1">
              <label className="inline-flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  aria-label={t("agent.confirm.preciseOrbitOption")}
                  checked={includePreciseOrbit}
                  disabled={proj.state !== "pending" || isAuto}
                  onChange={(e) => handlePreciseOrbitToggle(e.target.checked)}
                  className="rounded border-sp-line"
                />
                <span>{t("agent.confirm.companionOrbit")}</span>
              </label>
              {includePreciseOrbit && (
                <span className="text-xs text-sp-accent font-medium">
                  {t("agent.confirm.defaultPreciseOrbitNotice")}
                </span>
              )}
            </dd>
          </>
        )}

        {/* 保存位置（非 editable：task_action 或旧快照） */}
        {!row.editable && row.dir && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.confirm.saveLocation")}</dt>
            <dd
              className="m-0 text-sp-text font-mono text-xs break-all"
              title={row.dir}
            >
              {truncateMiddle(row.dir, 40)}
            </dd>
          </>
        )}

        {/* 下载目录（editable：create_task 快照） */}
        {row.editable && (
          <>
            <dt className="text-sp-text-muted whitespace-nowrap">{t("agent.confirm.downloadDirectory")}</dt>
            <dd className="m-0 text-sp-text flex flex-col gap-2">
              <div className="flex flex-wrap items-center gap-2">
                {row.dir ? (
                  <span
                    className="font-mono text-xs break-all text-sp-text"
                    title={row.dir}
                  >
                    {truncateMiddle(row.dir, 40)}
                  </span>
                ) : (
                  <span className="text-xs text-sp-text-muted">{t(approved ? "agent.unknown" : "agent.confirm.notSelected")}</span>
                )}
                {row.sourceLabel && (
                  <span className="rounded-full bg-sp-surface-muted text-sp-text-secondary border border-sp-line px-2 text-xs">
                    {row.sourceLabel}
                  </span>
                )}
              </div>

              {/* 只有在卡片待批且不是自动执行卡时显示操作 */}
              {proj.state === "pending" && !isAuto && (
                <>
                  {editingDir ? (
                    <div className="flex flex-wrap items-center gap-2 w-full">
                      <label className="sr-only" htmlFor={inputId}>
                        {t("agent.confirm.downloadDirectory")}
                      </label>
                      <input
                        id={inputId}
                        type="text"
                        className="h-8 px-2 rounded-sp-sm border border-sp-line bg-sp-canvas text-xs font-mono text-sp-text outline-none focus:ring-1 focus:ring-sp-accent flex-1 min-w-0"
                        placeholder={t("agent.confirm.directoryExample")}
                        value={dirDraft}
                        onChange={(e) => setDirDraft(e.target.value)}
                      />
                      <div className="flex items-center gap-2 shrink-0">
                        <button
                          type="button"
                          title={t("agent.confirm.confirmDirectory")}
                          onClick={() => {
                            const err = validateLocalOutputDir(dirDraft);
                            if (err) {
                              setDirError(err);
                            } else {
                              setChosenDir(dirDraft);
                              setDirError(null);
                              setEditingDir(false);
                            }
                          }}
                          className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp"
                        >
                          {t("agent.confirm.confirmDirectory")}
                        </button>
                        <button
                          type="button"
                          title={t("agent.cancel")}
                          onClick={() => {
                            setEditingDir(false);
                            setDirError(null);
                          }}
                          className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp"
                        >
                          {t("agent.cancel")}
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className="flex flex-wrap items-center gap-2">
                      <button
                        type="button"
                        title={t("agent.confirm.changeDirectory")}
                        onClick={async () => {
                          if (onPickDirectory) {
                            try {
                              const picked = await onPickDirectory();
                              if (picked !== null) {
                                const err = validateLocalOutputDir(picked);
                                if (err) {
                                  setDirError(err);
                                } else {
                                  setChosenDir(picked);
                                  setDirError(null);
                                }
                              }
                            } catch {
                              // 外层处理
                            }
                          } else {
                            setDirDraft(row.dir || "");
                            setDirError(null);
                            setEditingDir(true);
                          }
                        }}
                        className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp"
                      >
                        {t("agent.confirm.change")}
                      </button>
                      {defaultOffer && (
                        <button
                          type="button"
                          title={t("agent.confirm.useDefault")}
                          onClick={() => {
                            setChosenDir(defaultOffer);
                            setDirError(null);
                          }}
                          className="h-8 px-3 rounded-sp-sm border border-sp-line bg-sp-surface text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors duration-fast ease-sp"
                        >
                          {t("agent.confirm.useDefault")}
                        </button>
                      )}
                    </div>
                  )}

                  {dirError && (
                    <div className="text-xs text-sp-danger-text">
                      {dirError}
                    </div>
                  )}

                  {row.hint && (
                    <div className="text-xs text-sp-warning-text">
                      {row.hint}
                    </div>
                  )}
                </>
              )}
            </dd>
          </>
        )}
      </dl>

      {/* 需要确认的原因（card_reasons，按后端顺序） */}
      {hasReasons && (
        <div className="bg-sp-warning-bg text-sp-warning-text rounded-sp-md p-3 text-xs flex flex-col gap-1">
          <div className="font-medium">{t("agent.confirm.reasons")}</div>
          {approvalProj.reasonsZh.length > 0 && (
            <div className="flex flex-wrap gap-2 items-center">
              {approvalProj.reasonsZh.map((itemZh, idx) => (
                <span
                  key={idx}
                  className="px-2 py-1 rounded bg-sp-surface border border-sp-line text-xs font-medium text-sp-text"
                >
                  {itemZh}
                </span>
              ))}
            </div>
          )}
        </div>
      )}

      {/* 已批准确认卡关联任务卡（W5-8 P1-6） */}
      {approvedTaskModels.length > 0 && (
        <div className="flex flex-col gap-2 pt-1">
          {approvedTaskModels.map((m) => (
            <AgentTaskCard
              key={m.taskId}
              model={m}
              onTaskAction={onTaskAction}
            />
          ))}
        </div>
      )}

      {/* 底部操作区 */}
      <div className="cds-decision-footer flex items-center justify-between gap-3 pt-1 flex-wrap">
        <span className="text-xs text-sp-text-muted">
          {isAuto
            ? t("agent.confirm.autoNote")
            : t("agent.confirm.manualNote")}
        </span>

        <div className="flex items-center gap-2 ml-auto">
          {!isAuto && onViewOnMap && (
            <button
              type="button"
              onClick={onViewOnMap}
              className="cds-secondary-pill flex items-center gap-1.5 px-3 py-1.5 rounded-sp-sm text-xs font-medium text-sp-text cursor-pointer"
            >
              <MapPin className="h-3.5 w-3.5 text-sp-text-secondary" />
              <span>{t("agent.confirm.viewOnMap")}</span>
            </button>
          )}

          {isAuto ? (
            autoCancelOutcome ? (
              <span className="text-xs text-sp-text-muted font-medium">
                {autoCancelOutcome}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => onCancelAuto?.(item.run_id, createdTaskIds)}
                className="cds-secondary-pill h-8 px-3 rounded-sp-sm text-xs font-medium text-sp-text cursor-pointer"
              >
                {t("agent.cancel")}
              </button>
            )
          ) : (
            proj.state === "pending" && (
              <>
                <button
                  type="button"
                  onClick={() => onReject?.(item.plan_id)}
                  className="cds-btn-squish px-3 py-1.5 rounded-sp-sm bg-transparent text-xs font-medium text-sp-danger-text hover:bg-sp-danger-bg cursor-pointer transition-colors duration-fast ease-sp"
                >
                  {t("agent.confirm.reject")}
                </button>
                <button
                  type="button"
                  disabled={row.approveBlocked}
                  aria-disabled={row.approveBlocked ? "true" : undefined}
                  title={row.approveBlocked ? t("agent.confirm.selectDirectoryFirst") : undefined}
                  onClick={() => {
                    if (row.approveBlocked) return;
                    onApprove?.(
                      item.plan_id,
                      item.snapshot,
                      approveOutputDirArg(item.snapshot, chosenDir),
                      includePreciseOrbit,
                      isNisarProduct ? { nisarSubset: includeNisarSubset } : undefined
                    );
                  }}
                  className="cds-btn-squish h-8 px-3.5 gap-2 flex items-center rounded-sp-sm bg-sp-accent-fill text-white text-xs font-semibold shadow-card enabled:hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-opacity duration-fast ease-sp"
                >
                  <Download className="h-3.5 w-3.5" />
                  <span>{t("agent.confirm.enqueue")}</span>
                </button>
              </>
            )
          )}
        </div>
      </div>
      </div>
    </div>
  );
}
