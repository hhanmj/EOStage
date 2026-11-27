/**
 * approvalLogic.ts
 * 审批决策投影（自动执行 / 需确认原因 / 禁止档）、取消计算与错误卡投影纯逻辑模块
 * 规范：v3-contracts.ts 3.30.0、A1 §3.1/§3.1.1（W3f 三档）、EOStage 权限三档规范
 */

import type {
  AgentCardApproval,
  AgentApprovalDecision,
  AgentCardReason,
} from "../../lib/generated/v3-contracts.ts";
import { t } from "../../i18n/index.ts";
import { permissionModeLabelZh } from "./agentPermissionLogic.ts";
import { preferCodeExplanation } from "./errorExplanations.ts";
import type { ApplicationTask } from "./types.ts";

export type ProjectedApprovalKind =
  | "auto"
  | "reasons"
  | "manual"
  | "forbidden";

export interface ProjectedApproval {
  kind: ProjectedApprovalKind;
  modeLabel: string | null;
  reasonsZh: string[];
}

export const KNOWN_APPROVAL_DECISIONS: ReadonlySet<AgentApprovalDecision> =
  new Set(["manual", "auto_by_mode", "refused_forbidden"]);

/**
 * 弹卡原因中文（A1 §3.1 card_reasons；卡片按后端给出的顺序列出）。
 */
export const CARD_REASON_ZH: Readonly<Record<AgentCardReason, string>> = {
  get output_dir_unknown() { return t("shared.approval.reasons.output_dir_unknown"); },
  get output_dir_unverified() { return t("shared.approval.reasons.output_dir_unverified"); },
  get size_unknown() { return t("shared.approval.reasons.size_unknown"); },
  get reopens_existing_task() { return t("shared.approval.reasons.reopens_existing_task"); },
  get account_not_ready() { return t("shared.approval.reasons.account_not_ready"); },
  get selection_not_from_run() { return t("shared.approval.reasons.selection_not_from_run"); },
  get targets_user_task() { return t("shared.approval.reasons.targets_user_task"); },
  get archive_requires_confirm() { return t("shared.approval.reasons.archive_requires_confirm"); },
  get ledger_edit_requires_confirm() { return t("shared.approval.reasons.ledger_edit_requires_confirm"); },
  get tool_requires_confirm() { return t("shared.approval.reasons.tool_requires_confirm"); },
};

function isCardReason(value: unknown): value is AgentCardReason {
  return typeof value === "string" && Object.prototype.hasOwnProperty.call(CARD_REASON_ZH, value);
}

/**
 * 规范化 AgentCardApproval：
 * decision 不在三值内 -> null；card_reasons 只保留十个已知值；
 * W3e 旧事件的 exceeded 字段丢弃，旧 mode 值原样保留（显示时为「未知模式」）。
 */
export function normalizeCardApproval(
  raw: any
): AgentCardApproval | null {
  if (!raw || typeof raw !== "object") return null;

  const decision = raw.decision as AgentApprovalDecision;
  if (!KNOWN_APPROVAL_DECISIONS.has(decision)) {
    return null;
  }

  const mode = raw.mode as AgentCardApproval["mode"];
  const reasons = Array.isArray(raw.card_reasons)
    ? (raw.card_reasons as unknown[]).filter(isCardReason)
    : undefined;

  return {
    decision,
    mode,
    ...(reasons !== undefined ? { card_reasons: reasons } : {}),
  };
}

/**
 * 审批里记录的模式名：三档 -> 中文名；W3e 旧值或其他字符串 -> 「未知模式」；没有 mode -> null。
 */
export function approvalModeLabelZh(mode: unknown): string | null {
  if (typeof mode !== "string" || mode.length === 0) return null;
  return permissionModeLabelZh(mode) ?? t("shared.approval.unknownMode");
}

/**
 * 投影审批状态
 * - decision === "refused_forbidden" 或 errorCode === "plan_refused_forbidden" -> forbidden
 * - decision === "auto_by_mode" 且 autoExecuted === true -> auto
 * - decision === "manual" 且 card_reasons 非空 -> reasons（卡片列出原因）
 * - 其余（没有 approval 的旧事件、auto_by_mode 但 auto_executed 不为 true 的不一致情况）-> manual（保守：保留人工按钮）
 */
export function projectApproval(
  approval?: AgentCardApproval | null,
  autoExecuted?: boolean,
  errorCode?: string | null
): ProjectedApproval {
  const normApproval = approval ? normalizeCardApproval(approval) : null;
  const decision = normApproval?.decision;

  const reasonsZh: string[] = [];
  for (const reason of normApproval?.card_reasons ?? []) {
    reasonsZh.push(CARD_REASON_ZH[reason]);
  }

  let kind: ProjectedApprovalKind = "manual";
  if (decision === "refused_forbidden" || errorCode === "plan_refused_forbidden") {
    kind = "forbidden";
  } else if (decision === "auto_by_mode" && autoExecuted === true) {
    kind = "auto";
  } else if (decision === "manual" && reasonsZh.length > 0) {
    kind = "reasons";
  }

  return {
    kind,
    modeLabel: approvalModeLabelZh(normApproval?.mode),
    reasonsZh,
  };
}

export interface PlanAutoCancelOptions {
  runId: string;
  runLive: boolean;
  createdTaskIds: string[];
  tasks: ApplicationTask[];
}

export interface PlanAutoCancelResult {
  cancelRun: boolean;
  cancelTasks: { taskId: string; revision: number }[];
  skipped: { taskId: string; reason: "not_found" | "not_active" }[];
}

const ACTIVE_TASK_STATUSES = new Set([
  "queued",
  "running",
  "paused",
  "interrupted",
]);

/**
 * 计算自动执行计划的取消目标：
 * runLive 为真才取消运行；
 * created 任务在 tasks 里且状态 queued/running/paused/interrupted -> 取消（带 revision）；
 * 不在 tasks -> not_found；已结束 -> not_active。
 */
export function planAutoCancel(
  options: PlanAutoCancelOptions
): PlanAutoCancelResult {
  const { runLive, createdTaskIds = [], tasks = [] } = options;
  const cancelRun = Boolean(runLive);

  const taskMap = new Map<string, ApplicationTask>();
  for (const t of tasks) {
    if (t && t.task_id) {
      taskMap.set(t.task_id, t);
    }
  }

  const cancelTasks: { taskId: string; revision: number }[] = [];
  const skipped: { taskId: string; reason: "not_found" | "not_active" }[] = [];

  for (const rawId of createdTaskIds) {
    if (typeof rawId !== "string" || !rawId.trim()) continue;
    const taskId = rawId.trim();
    const task = taskMap.get(taskId);

    if (!task) {
      skipped.push({ taskId, reason: "not_found" });
    } else if (ACTIVE_TASK_STATUSES.has(task.status)) {
      cancelTasks.push({ taskId, revision: task.revision });
    } else {
      skipped.push({ taskId, reason: "not_active" });
    }
  }

  return {
    cancelRun,
    cancelTasks,
    skipped,
  };
}

export interface AutoCancelOutcomeTextOptions {
  runCancelled: boolean;
  cancelledTaskIds: string[];
  failedTaskIds?: string[];
  skipped?: { taskId: string; reason: "not_found" | "not_active" }[];
}

/**
 * 取消操作结果中文文案：
 * 全部成功「已取消本次运行和 N 个任务」；
 * 没有运行可取消时「已取消 N 个任务」；
 * 什么都不用取消时「本次运行与任务都已结束，无需取消」；
 * 有失败或 not_found 时追加「未能取消：<任务名或 id>」（任务名取 tasks 的 name，找不到用 id）。
 */
export function autoCancelOutcomeText(
  options: AutoCancelOutcomeTextOptions,
  tasks?: ApplicationTask[]
): string {
  const {
    runCancelled,
    cancelledTaskIds = [],
    failedTaskIds = [],
    skipped = [],
  } = options;

  // 整理未能取消的任务 ID：显式失败的 + not_found 的
  const uncancelledIds: string[] = [];
  const seen = new Set<string>();

  for (const fid of failedTaskIds) {
    if (typeof fid === "string" && fid.trim() && !seen.has(fid.trim())) {
      const trimmed = fid.trim();
      seen.add(trimmed);
      uncancelledIds.push(trimmed);
    }
  }

  for (const sk of skipped) {
    if (sk.reason === "not_found" && !seen.has(sk.taskId)) {
      seen.add(sk.taskId);
      uncancelledIds.push(sk.taskId);
    }
  }

  const taskNameMap = new Map<string, string>();
  if (Array.isArray(tasks)) {
    for (const t of tasks) {
      if (t && t.task_id && t.name) {
        const trimmed = t.name.trim();
        if (trimmed) {
          taskNameMap.set(t.task_id, trimmed);
        }
      }
    }
  }

  const failedLabels = uncancelledIds.map((id) => taskNameMap.get(id) || id);

  let mainText = "";
  if (runCancelled && cancelledTaskIds.length > 0) {
    mainText = t("shared.approval.cancelledRunAndTasks", { count: cancelledTaskIds.length });
  } else if (!runCancelled && cancelledTaskIds.length > 0) {
    mainText = t("shared.approval.cancelledTasksOnly", { count: cancelledTaskIds.length });
  } else if (runCancelled && cancelledTaskIds.length === 0) {
    mainText = t("shared.approval.cancelledRunOnly");
  } else {
    // !runCancelled && cancelledTaskIds.length === 0
    if (failedLabels.length === 0) {
      mainText = t("shared.approval.alreadyEndedNoNeed");
    }
  }

  if (failedLabels.length > 0) {
    const failurePart = t("shared.approval.cancelFailedPart", {
      labels: failedLabels.join(t("shared.approval.listSeparator")),
    });
    return mainText
      ? `${mainText}${t("shared.approval.clauseSeparator")}${failurePart}`
      : failurePart;
  }

  return mainText;
}

export interface ProjectedErrorCard {
  wording: string;
  modeLine: string | null;
  detailCode: string | null;
}

/**
 * 错误卡投影（W5-2f R7，agnes P1）：可见文字只用中文解释，错误码原文只进「展开详情」。
 * W5-2g：额度已删除，旧事件的 exceeded 不再列出。
 * - 已取消 / 已中断：只显示中文状态，无详情；
 * - 禁止档（code 或 approval.decision）：固定说明 + 「当前模式：<模式名>」；
 * - 其余：事件带 message_zh（主机自己结束运行的原因，W3k）时显示它，否则 errorExplanations
 *   的中文解释，未知码显示「执行出错」。
 */
export function projectErrorCard(item: {
  code: string;
  status: string | null;
  approval?: AgentCardApproval | null;
  message_zh?: string | null;
}): ProjectedErrorCard {
  const approvalProj = projectApproval(item.approval, false, item.code);
  if (item.code === "cancelled" || item.status === "cancelled" || item.status === "interrupted") {
    return {
      wording: item.status === "interrupted" ? t("shared.approval.statusInterrupted") : t("shared.approval.statusCancelled"),
      modeLine: null,
      detailCode: null,
    };
  }
  const detailCode = item.code ? item.code : null;
  if (approvalProj.kind === "forbidden") {
    return {
      wording: t("shared.approval.forbiddenTierWording"),
      modeLine: approvalProj.modeLabel ? t("shared.approval.currentModeLine", { modeLabel: approvalProj.modeLabel }) : null,
      detailCode,
    };
  }
  return {
    wording: preferCodeExplanation(item.message_zh?.trim(), item.code) || t("shared.approval.executionFailedFallback"),
    modeLine: null,
    detailCode,
  };
}

/**
 * 错误卡的附加说明：输出超长被截断、且该运行的渠道拒绝了「关闭思考」时，
 * 说明思考内容占用了输出上限（L4 r9 P1-2）。其他情况没有说明。
 */
export function errorCardHintZh(code: string, reasoningOffRefused: boolean): string | null {
  if (code === "model_truncated" && reasoningOffRefused === true) {
    return t("shared.approval.thinkingCannotDisableNotice");
  }
  return null;
}
