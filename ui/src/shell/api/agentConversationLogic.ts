/**
 * Agent 对话态与卡片投影纯函数逻辑模块
 * 规范：W5-2b
 */

import type {
  StreamToolGroupItem,
  StreamConfirmCardItem,
  StreamProgressCardItem,
  StreamConclusionItem,
} from "./agentStreamLogic.ts";
import { confirmSnapshotOf, formatBytes } from "./agentStreamLogic.ts";
import type { ApplicationTask, TaskItemAction } from "./types.ts";
import { taskCardSizeLine } from "./taskCardLogic.ts";
import { getAvailableTaskActions, taskOutputDir } from "./taskLogic.ts";
import { formatOrbitTerms } from "./orbitTerms.ts";
import { mappedSourceName } from "./sourceDisplayName.ts";
import { t } from "../../i18n/index.ts";
import { dateOnly } from "./searchCardSource.ts";

function formatLocalTime(isoStr: string | null | undefined): string | null {
  if (!isoStr || typeof isoStr !== "string") return null;
  const d = new Date(isoStr);
  if (isNaN(d.getTime())) return null;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function formatDurationZh(ms: number | null | undefined): string | null {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < 1000) {
    return t("agent.conversation.milliseconds", { count: Math.round(ms) });
  }
  if (ms < 60000) {
    return t("agent.conversation.seconds", { count: (ms / 1000).toFixed(1) });
  }
  const totalSec = Math.floor(ms / 1000);
  if (totalSec < 3600) {
    const mins = Math.floor(totalSec / 60);
    const secs = totalSec % 60;
    return t("agent.conversation.minutesSeconds", { minutes: mins, seconds: secs });
  }
  const hours = Math.floor(totalSec / 3600);
  const mins = Math.floor((totalSec % 3600) / 60);
  const secs = totalSec % 60;
  return t("agent.conversation.hoursMinutesSeconds", {
    hours,
    minutes: mins,
    seconds: secs,
  });
}

export interface ProjectedToolGroupRow {
  name_zh: string;
  status: string;
  timeZh: string | null;
  durationZh: string | null;
  resultCountZh: string | null;
  summaryZh: string | null;
  targetLabel?: string | null;
  inputSummary?: string | null;
  outputSummary?: string | null;
}

export interface ProjectedToolGroup {
  count: number;
  status: string;
  rows: ProjectedToolGroupRow[];
}

export interface ProjectedSearchCard {
  status: string;
  source: string | null;
  conditionBadges: string[];
  resultCountZh: string | null;
}

/**
 * 投射检索卡事实与状态
 */
export function projectSearchCard(
  group: StreamToolGroupItem,
  snapshot: { source: string | null; conditionBadges: string[] } | null
): ProjectedSearchCard {
  let resultCountZh: string | null = null;
  for (let i = (group.rounds || []).length - 1; i >= 0; i--) {
    const r = group.rounds[i];
    if (
      r.tool_id === "search_progress" &&
      typeof r.result_count === "number" &&
      Number.isInteger(r.result_count) &&
      r.result_count >= 0
    ) {
      resultCountZh = t("agent.conversation.scenes", { count: r.result_count });
      break;
    }
  }

  return {
    status: group.status,
    source: snapshot ? snapshot.source : null,
    conditionBadges: snapshot ? snapshot.conditionBadges : [],
    resultCountZh,
  };
}

export interface ProjectToolGroupOptions {
  reasoningResent?: boolean;
}

/**
 * 投射执行过程工具组
 */
export function projectToolGroup(
  item: StreamToolGroupItem,
  resolveToolName?: (toolId: string) => string,
  options?: ProjectToolGroupOptions
): ProjectedToolGroup {
  const rows: ProjectedToolGroupRow[] = (item.rounds || []).map((r) => {
    const timeZh = formatLocalTime(r.started_at);
    const durationZh = formatDurationZh(r.duration_ms);
    const resultCountZh =
      typeof r.result_count === "number" &&
      Number.isInteger(r.result_count) &&
      r.result_count >= 0
        ? t("agent.conversation.returned", { count: r.result_count })
        : null;
    const summaryZh =
      typeof r.summary_zh === "string" && r.summary_zh.trim().length > 0
        ? r.summary_zh
        : null;
    const raw = r as Record<string, unknown>;
    const targetLabel =
      typeof raw.target_label === "string" && raw.target_label.trim().length > 0
        ? raw.target_label.trim()
        : null;
    const inputSummary =
      typeof raw.input_summary === "string" && raw.input_summary.trim().length > 0
        ? raw.input_summary.trim()
        : null;
    const outputSummary =
      typeof raw.output_summary === "string" && raw.output_summary.trim().length > 0
        ? raw.output_summary.trim()
        : null;

    return {
      name_zh: resolveToolName ? resolveToolName(r.tool_id) : r.tool_id,
      status: r.status,
      timeZh,
      durationZh,
      resultCountZh,
      summaryZh,
      ...(targetLabel ? { targetLabel } : {}),
      ...(inputSummary ? { inputSummary } : {}),
      ...(outputSummary ? { outputSummary } : {}),
    };
  });

  if (options?.reasoningResent === true) {
    rows.push({
      name_zh: t("agent.conversation.reasoningResent"),
      status: "done",
      timeZh: null,
      durationZh: null,
      resultCountZh: null,
      summaryZh: null,
    });
  }

  return {
    count: rows.length,
    status: item.status,
    rows,
  };
}

export interface OrbitBadgeItem {
  label: string;
  tone: "asc" | "desc" | "other";
}

export interface ProjectedConfirmCard {
  source: string;
  conditionBadges: string[];
  aoiLine: string | null;
  estimatedSizeZh: string;
  outputDir: string | null;
  state: "pending" | "approved" | "rejected" | "expired" | "superseded";
  resultLine: string | null;
  orbitBadges: OrbitBadgeItem[];
}

/**
 * 投射确认卡事实与状态迁移
 */
export function projectConfirmCard(
  item: StreamConfirmCardItem,
  subsequent: StreamProgressCardItem | null
): ProjectedConfirmCard {
  const extracted = confirmSnapshotOf(item.snapshot);

  const conditionBadges: string[] = [];
  if (extracted.range.product && extracted.range.product !== "未知") { // i18n-exempt: backend snapshot sentinel
    conditionBadges.push(extracted.range.product);
  }
  if (extracted.range.time_range) {
    const start = dateOnly(extracted.range.time_range.start);
    const end = dateOnly(extracted.range.time_range.end);
    if (start && end) conditionBadges.push(t("agent.conversation.range", { start, end }));
    else if (start) conditionBadges.push(t("agent.conversation.rangeFrom", { start }));
    else if (end) conditionBadges.push(t("agent.conversation.rangeUntil", { end }));
  } else if (
    extracted.range.time_range_zh &&
    extracted.range.time_range_zh !== "未知" // i18n-exempt: backend snapshot sentinel
  ) {
    conditionBadges.push(extracted.range.time_range_zh);
  }

  let aoiLine: string | null = null;
  const hasAoiName =
    extracted.range.aoi_name && extracted.range.aoi_name !== "未知"; // i18n-exempt: backend snapshot sentinel
  const hasArea =
    typeof extracted.range.area_km2 === "number" &&
    !isNaN(extracted.range.area_km2) &&
    extracted.range.area_km2 > 0;

  if (hasAoiName && hasArea) {
    aoiLine = t("agent.conversation.aoiArea", { name: extracted.range.aoi_name, area: Math.round(extracted.range.area_km2!) });
  } else if (hasAoiName) {
    aoiLine = extracted.range.aoi_name;
  } else if (hasArea) {
    aoiLine = t("agent.conversation.area", { area: Math.round(extracted.range.area_km2!) });
  }

  let state: "pending" | "approved" | "rejected" | "expired" | "superseded" = "pending";
  if (subsequent) {
    if (subsequent.code === "plan_rejected" || subsequent.status === "rejected") {
      state = "rejected";
    } else if (
      typeof subsequent.code === "string" &&
      subsequent.code.startsWith("plan_superseded")
    ) {
      state = "superseded";
    } else if (subsequent.code === "plan_expired" || subsequent.status === "expired") {
      state = "expired";
    } else if (
      subsequent.code === "plan_approved" ||
      subsequent.status === "running" ||
      subsequent.status === "failed" ||
      subsequent.status === "cancelled" ||
      subsequent.status === "interrupted" ||
      subsequent.status === "done" ||
      subsequent.status === "approved" ||
      subsequent.status === "completed"
    ) {
      state = "approved";
    }
  }

  const outputDir =
    extracted.output_dir && extracted.output_dir !== "未知" // i18n-exempt: backend snapshot sentinel
      ? extracted.output_dir
      : null;

  let resultLine: string | null = null;
  const resultCount = item.snapshot?.result_count;
  const alreadyQueued = item.snapshot?.already_queued_count;
  if (
    typeof resultCount === "number" &&
    Number.isInteger(resultCount) &&
    resultCount >= 0
  ) {
    if (
      typeof alreadyQueued === "number" &&
      Number.isInteger(alreadyQueued) &&
      alreadyQueued > 0
    ) {
      resultLine = t("agent.conversation.queuedAgain", { count: resultCount, queued: alreadyQueued });
    } else if (alreadyQueued === 0) {
      resultLine = t("agent.conversation.newScenes", { count: resultCount });
    } else {
      resultLine = t("agent.conversation.scenes", { count: resultCount });
    }
  }

  const orbitBadges: OrbitBadgeItem[] = [];
  const seenOrbits = new Set<string>();
  if (Array.isArray(item.snapshot?.orbit_directions)) {
    for (const d of item.snapshot.orbit_directions) {
      if (typeof d === "string" && d.trim().length > 0 && !seenOrbits.has(d)) {
        seenOrbits.add(d);
        let tone: "asc" | "desc" | "other" = "other";
        if (d === "Ascending") tone = "asc";
        else if (d === "Descending") tone = "desc";
        orbitBadges.push({ label: d, tone });
      }
    }
  }

  return {
    source: extracted.source || t("agent.conversation.unknownSource"),
    conditionBadges: conditionBadges.map(formatOrbitTerms),
    aoiLine,
    estimatedSizeZh: extracted.estimated_bytes_zh || t("agent.conversation.unknownSize"),
    outputDir,
    state,
    resultLine,
    orbitBadges,
  };
}

export interface ProjectedEvidenceRef {
  kind: string;
  kind_zh: string;
  id: string;
  label: string;
  verified: boolean | null;
  reasonZh?: string;
  traceZh: string | null;
  responseZh: string | null;
}

export interface ProjectedEvidence {
  runId: string;
  verified: boolean;
  status: "verified" | "partial" | "unverified" | "uncited";
  refs: ProjectedEvidenceRef[];
  refCount: number;
  summaryTraceZh: string | null;
  summaryResponseZh: string | null;
}

/**
 * 投射证据行及其引用展开列表
 */
export function projectEvidence(
  item: StreamConclusionItem
): ProjectedEvidence {
  const claims = Array.isArray(item.claims) ? item.claims : [];

  const citedClaims = claims.filter((claim) => {
    if (!claim || typeof claim !== "object") return false;
    const refs = (claim as { refs?: unknown }).refs;
    return Array.isArray(refs) && refs.length > 0;
  });
  const verifiedClaimCount = citedClaims.filter((claim) => isVerifiedEvidenceClaim(claim)).length;
  const status: "verified" | "partial" | "unverified" | "uncited" =
    claims.length === 0
      ? "uncited"
      : citedClaims.length > 0 && verifiedClaimCount === claims.length
        ? "verified"
        : verifiedClaimCount > 0
          ? "partial"
          : "unverified";
  const verified = status === "verified";

  const refs: ProjectedEvidenceRef[] = [];
  let summaryTraceZh: string | null = null;
  let summaryResponseZh: string | null = null;

  for (const claim of claims) {
    if (claim && typeof claim === "object" && Array.isArray(claim.refs)) {
      for (const r of claim.refs) {
        if (!isEvidenceRef(r)) continue;
        let kind_zh = t("agent.conversation.relatedReference");
        if (r.kind === "task") kind_zh = t("agent.conversation.relatedTask");
        else if (r.kind === "batch") kind_zh = t("agent.conversation.relatedBatch");
        else if (r.kind === "web") kind_zh = t("agent.conversation.webSource");

        const traceZh =
          typeof r.trace_sequence === "number" &&
          Number.isInteger(r.trace_sequence) &&
          r.trace_sequence > 0
            ? t("agent.conversation.trace", { count: r.trace_sequence })
            : null;
        const responseZh =
          typeof r.response_count === "number" &&
          Number.isInteger(r.response_count) &&
          r.response_count >= 0
            ? t("agent.conversation.responses", { count: r.response_count })
            : null;

        if (traceZh && summaryTraceZh === null) {
          summaryTraceZh = traceZh;
        }
        if (responseZh && summaryResponseZh === null) {
          summaryResponseZh = responseZh;
        }

        refs.push({
          kind: r.kind,
          kind_zh,
          id: r.id,
          label: r.label || r.id,
          verified: isVerifiedEvidenceClaim(claim) ? true : null,
          ...(!isVerifiedEvidenceClaim(claim) ? { reasonZh: claim.verified === false ? t("agent.conversation.claimRefUnknown") : t("agent.conversation.claimDataUnknown") } : {}),
          traceZh,
          responseZh,
        });
      }
    }
  }

  return {
    runId: item.run_id,
    verified,
    status,
    refs,
    refCount: refs.length,
    summaryTraceZh,
    summaryResponseZh,
  };
}

export interface EvidenceSummary {
  status: "verified" | "partial" | "unverified" | "uncited";
  labelZh: string;
  reasonZh: string | null;
  uncitedClaims: number;
}

function isVerifiedEvidenceClaim(claim: unknown): boolean {
  if (!claim || typeof claim !== "object") return false;
  const candidate = claim as { verified?: unknown; refs?: unknown };
  return candidate.verified === true && Array.isArray(candidate.refs) && candidate.refs.length > 0 && candidate.refs.every(isEvidenceRef);
}

function isEvidenceRef(ref: unknown): boolean {
  if (!ref || typeof ref !== "object") return false;
  const candidate = ref as { kind?: unknown; id?: unknown };
  return typeof candidate.kind === "string" && candidate.kind.trim().length > 0 &&
    typeof candidate.id === "string" && candidate.id.trim().length > 0;
}

function normalizedEvidenceClaims(item: StreamConclusionItem): Array<{ verified: unknown; refs: unknown[]; malformed: boolean }> {
  if (!Array.isArray(item.claims)) return [];
  return item.claims.map((claim) => {
    if (!claim || typeof claim !== "object") return { verified: undefined, refs: [], malformed: true };
    const candidate = claim as { verified?: unknown; refs?: unknown };
    return {
      verified: candidate.verified,
      refs: Array.isArray(candidate.refs) ? candidate.refs : [],
      malformed: !Array.isArray(candidate.refs),
    };
  });
}

/** 只收到陈述级核验结果，不能推断具体哪条引用失败。 */
export function evidenceSummary(item: StreamConclusionItem): EvidenceSummary {
  const claims = normalizedEvidenceClaims(item);
  const uncitedClaims = claims.filter((claim) => !claim.malformed && claim.refs.length === 0).length;
  const malformedClaims = claims.filter((claim) => claim.malformed).length;
  if (claims.length === 0) {
    return { status: "uncited", labelZh: t("agent.conclusion.uncited"), reasonZh: null, uncitedClaims };
  }
  const citedClaims = claims.filter((claim) => !claim.malformed && claim.refs.length > 0);
  const verifiedClaimCount = citedClaims.filter((claim) => claim.verified === true && claim.refs.every(isEvidenceRef)).length;
  if (claims.length > 0 && malformedClaims === 0 && uncitedClaims === 0 && citedClaims.length === claims.length && verifiedClaimCount === claims.length) {
    return { status: "verified", labelZh: t("agent.conclusion.verified"), reasonZh: null, uncitedClaims };
  }
  const reason = citedClaims.length > 0
    ? t("agent.conversation.citedReason", { cited: citedClaims.length, verified: verifiedClaimCount })
    : null;
  const uncitedReason = uncitedClaims > 0 ? t("agent.conversation.uncitedReason", { count: uncitedClaims }) : null;
  const parts = [reason, uncitedReason && (reason ? t("agent.conversation.alsoUncited", { count: uncitedReason }) : uncitedReason), malformedClaims > 0 ? t("agent.conversation.malformedReason", { count: malformedClaims }) : null].filter(Boolean);
  return {
    status: verifiedClaimCount > 0 ? "partial" : "unverified",
    labelZh: verifiedClaimCount > 0 ? t("agent.conversation.partiallyVerified") : t("agent.conclusion.unverified"),
    reasonZh: parts.join(t("agent.conversation.reasonSeparator")) || null,
    uncitedClaims,
  };
}

/** 主机只提供陈述整体结果，不提供逐引用失败原因。 */
export function evidenceRefReasonZh(
  _ref: { kind: string; trace_sequence?: number | null },
  _claimRefCount = 1
): string {
  return t("agent.conversation.claimRefUnknown");
}

/**
 * 规范任务生命周期中文标签映射（避免出现底层实现枚举）
 */
export function taskStatusLabelZh(status: string): string {
  switch (status) {
    case "queued":
      return t("agent.status.queued");
    case "running":
      return t("agent.status.transferring");
    case "paused":
      return t("agent.status.paused");
    case "completed":
      return t("agent.status.completed");
    case "failed":
      return t("agent.status.failed");
    case "cancelled":
      return t("agent.status.cancelled");
    case "interrupted":
      return t("agent.status.interrupted");
    default:
      return status;
  }
}

export interface UnknownTaskCardSummary {
  name: string;
  status?: undefined;
  revision?: undefined;
  category?: undefined;
  updated_at?: undefined;
  pause_reason?: undefined;
}

export interface TaskCardModel {
  taskId: string;
  statusKnown?: boolean;
  task: ApplicationTask | UnknownTaskCardSummary;
  productLine: string | null;
  percent: number | null;
  sizeLine: string;
  remainingMinutesZh: string | null;
  statusLabel: string;
  actions: TaskItemAction[];
}

/**
 * 根据引用与 W4 任务投影构建任务卡片展示模型（未匹配的任务引用绝不捏造卡片）
 */
export function buildTaskCardModels(
  refs: Array<{ id: string; label?: string; kind?: string }>,
  tasks: ApplicationTask[]
): TaskCardModel[] {
  if (!Array.isArray(refs) || !Array.isArray(tasks) || tasks.length === 0) {
    return [];
  }

  const models: TaskCardModel[] = [];
  const seenTaskIds = new Set<string>();

  for (const ref of refs) {
    const matchedTask = tasks.find(
      (t) => t.task_id === ref.id || (t as any).id === ref.id
    );
    if (!matchedTask) continue;

    const taskId = matchedTask.task_id || (matchedTask as any).id;
    if (seenTaskIds.has(taskId)) continue;
    seenTaskIds.add(taskId);

    const hasTotal =
      typeof matchedTask.total_bytes === "number" &&
      matchedTask.total_bytes > 0 &&
      matchedTask.bytes_reported !== false;
    const percent = hasTotal
      ? Math.min(
          100,
          Math.max(
            0,
            Math.round((matchedTask.done_bytes / matchedTask.total_bytes!) * 100)
          )
        )
      : null;

    const sizeLine = taskCardSizeLine(matchedTask, (b) => formatBytes(b ?? 0));

    let remainingMinutesZh: string | null = null;
    if (
      typeof matchedTask.bytes_per_second === "number" &&
      matchedTask.bytes_per_second > 0 &&
      hasTotal &&
      matchedTask.total_bytes! > matchedTask.done_bytes
    ) {
      const remainingSecs =
        (matchedTask.total_bytes! - matchedTask.done_bytes) /
        matchedTask.bytes_per_second;
      const mins = Math.max(1, Math.round(remainingSecs / 60));
      remainingMinutesZh = t("agent.conversation.remaining", { count: mins });
    }

    const statusLabel = taskStatusLabelZh(matchedTask.status);
    const actions = getAvailableTaskActions(
      matchedTask.status,
      taskOutputDir(matchedTask) !== null
    );

    const sceneIds = matchedTask.scene_ids;
    let productLine: string | null = null;
    if (Array.isArray(sceneIds) && sceneIds.length > 0) {
      if (sceneIds.length === 1) {
        productLine = sceneIds[0];
      } else {
        productLine = t("agent.conversation.productSeries", { name: sceneIds[0], count: sceneIds.length });
      }
    }

    models.push({
      taskId,
      statusKnown: true,
      task: matchedTask,
      productLine,
      percent,
      sizeLine,
      remainingMinutesZh,
      statusLabel,
      actions,
    });
  }

  return models;
}


const TERMINAL_PROGRESS_STATUSES = new Set([
  "done",
  "approved",
  "rejected",
  "expired",
  "failed",
  "cancelled",
  "interrupted",
]);

const TERMINAL_PROGRESS_CODES = new Set([
  "plan_approved",
  "plan_rejected",
  "plan_expired",
  "plan_superseded_by_new_turn",
  "plan_superseded_by_aoi_change",
]);

/**
 * 在确认卡或计划卡之后的事件流项中，选取指定 plan_id 的终态（最新）progress_card（W5-8 P1-6）。
 * 若存在终态项，返回最后一张终态项；否则返回最后一张匹配的 progress_card（如 running），无匹配返回 null。
 */
export function findTerminalProgressForPlan(
  items: ReadonlyArray<{
    kind?: string;
    plan_id?: string | null;
    status?: string | null;
    code?: string | null;
    created_task_ids?: string[];
  }>,
  planId: string
): StreamProgressCardItem | null {
  if (!Array.isArray(items) || !planId) return null;
  const matching = items.filter(
    (it): it is StreamProgressCardItem =>
      Boolean(
        it &&
          (it.kind === "progress_card" || !("kind" in it)) &&
          it.plan_id === planId
      )
  );
  if (matching.length === 0) return null;

  for (let i = matching.length - 1; i >= 0; i--) {
    const candidate = matching[i];
    if (
      (candidate.code && TERMINAL_PROGRESS_CODES.has(candidate.code)) ||
      (candidate.status && TERMINAL_PROGRESS_STATUSES.has(candidate.status))
    ) {
      return candidate;
    }
  }
  return matching[matching.length - 1] ?? null;
}

/**
 * 为已批准确认卡构建关联任务卡模型列表（W5-8 P1-6 / R4c-1）：
 * - 若 tasks 中存在匹配任务，优先复用 buildTaskCardModels；产品行只用景名，未知时为 null（任务 ID 不显示）；
 * - 若 tasks 尚未同步或独立渲染，按 createdTaskIds 诚实生成真正的「状态未知」只读任务卡模型（不猜 status、不带 revision、不显示任务 ID 或 resource_id 原文）。
 */
export function buildConfirmCardTaskModels(
  createdTaskIds: ReadonlyArray<string>,
  tasks: ApplicationTask[] = [],
  fallbackMeta?: {
    source?: string | null;
    sourceNames?: Readonly<Record<string, string>> | null;
    /** 已解析的来源显示名（确认卡「来源」同一值）；「未知」视为没有 */
    sourceLabel?: string | null;
    estimatedSizeZh?: string;
    outputDir?: string | null;
  }
): TaskCardModel[] {
  if (!Array.isArray(createdTaskIds) || createdTaskIds.length === 0) {
    return [];
  }
  const models: TaskCardModel[] = [];
  const seen = new Set<string>();

  for (const rawId of createdTaskIds) {
    if (typeof rawId !== "string" || !rawId.trim()) continue;
    const taskId = rawId.trim();
    if (seen.has(taskId)) continue;
    seen.add(taskId);

    const matched = buildTaskCardModels([{ id: taskId, kind: "task" }], tasks);
    if (matched.length > 0) {
      const m = matched[0];
      models.push({
        ...m,
        statusKnown: true,
        productLine: m.productLine,
      });
    } else {
      const label = fallbackMeta?.sourceLabel?.trim();
      const displayName =
        fallbackMeta?.sourceLabel !== undefined
          ? label && label !== "未知" // i18n-exempt: existing source-label sentinel
            ? label
            : null
          : mappedSourceName(fallbackMeta?.source, fallbackMeta?.sourceNames);
      const fallbackTitle = displayName ? t("agent.conversation.namedDownloadTask", { name: displayName }) : t("agent.conversation.downloadTask");
      models.push({
        taskId,
        statusKnown: false,
        task: {
          name: fallbackTitle,
        },
        productLine: null,
        percent: null,
        sizeLine: fallbackMeta?.estimatedSizeZh || t("agent.conversation.unknownSize"),
        remainingMinutesZh: null,
        statusLabel: t("agent.conversation.unknownStatus"),
        actions: [],
      });
    }
  }
  return models;
}

/**
 * 根据计划卡步骤自身状态与后续 progress_card 最新状态，计算步骤展示状态（W5-8 P2-1）
 */
export function projectPlanStepStatus(
  stepStatus: string,
  progressStatus?: string | null,
  progressCode?: string | null
): string {
  if (progressCode === "plan_rejected" || progressStatus === "rejected") {
    return "rejected";
  }
  if (progressCode === "plan_expired" || progressStatus === "expired") {
    return "expired";
  }
  if (progressStatus === "cancelled") {
    return "cancelled";
  }
  if (progressStatus === "interrupted") {
    return "interrupted";
  }
  if (progressStatus === "failed") {
    return "failed";
  }
  if (progressCode === "plan_approved" || progressStatus === "approved") {
    return "approved";
  }
  if (progressStatus === "done" || progressStatus === "completed") {
    return "done";
  }
  if (progressStatus === "running") {
    return "running";
  }
  return stepStatus || "pending";
}

/**
 * 计划步骤状态中文文案映射（已完成/待执行/执行中/失败/已取消/已中断/已批准/已拒绝/已失效；
 * done 与进度卡、检索步骤一致写「已完成」）
 */
export function planStepStatusLabelZh(status: string): string {
  switch (status) {
    case "done":
      return t("agent.status.completed");
    case "pending":
      return t("agent.status.pending");
    case "running":
      return t("agent.status.running");
    case "failed":
      return t("agent.status.failure");
    case "cancelled":
      return t("agent.status.cancelled");
    case "interrupted":
      return t("agent.status.interrupted");
    case "approved":
      return t("agent.status.approved");
    case "rejected":
      return t("agent.status.rejected");
    case "expired":
      return t("agent.status.expired");
    default:
      return t("agent.status.pending");
  }
}

/**
 * 进度卡状态中文映射（W5-8 P2-1：严禁将英文枚举原值直接暴露到界面）
 */
export function progressStatusLabelZh(status?: string | null): string {
  switch (status) {
    case "running":
      return t("agent.status.running");
    case "done":
    case "completed":
      return t("agent.status.completed");
    case "failed":
      return t("agent.status.failure");
    case "cancelled":
      return t("agent.status.cancelled");
    case "interrupted":
      return t("agent.status.interrupted");
    case "queued":
    case "pending":
      return t("agent.status.queued");
    case "paused":
      return t("agent.status.paused");
    case "approved":
      return t("agent.status.approved");
    case "rejected":
      return t("agent.status.rejected");
    case "expired":
      return t("agent.status.expired");
    default:
      return t("agent.unknown");
  }
}

/**
 * 进度卡状态行完整中文文案
 */
export function progressStatusLineZh(
  item: { status?: string | null; code?: string | null } | string | null | undefined
): string {
  if (item && typeof item === "object") {
    if (item.code === "plan_approved") {
      return t("agent.conversation.approvedProgress");
    }
    if (item.code === "plan_rejected") {
      return t("agent.conversation.rejectedProgress");
    }
    if (item.code === "plan_expired") {
      return t("agent.conversation.expiredProgress");
    }
    if (typeof item.code === "string" && item.code.startsWith("plan_superseded")) {
      return t("agent.confirm.superseded");
    }
    return t("agent.conversation.statusLine", { status: progressStatusLabelZh(item.status) });
  }
  return t("agent.conversation.statusLine", { status: progressStatusLabelZh(typeof item === "string" ? item : null) });
}
