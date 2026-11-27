import type { OrbitRecheckInfo } from "../../lib/generated/v3-contracts.ts";
import { t } from "../../i18n/index.ts";
import { formatErrorWithCode, preferCodeExplanation } from "./errorExplanations.ts";
import { isGacosWebformTask } from "./taskLogic.ts";
import type { ApplicationTask, TaskSceneItem } from "./types.ts";

/** The companion orbit task's `auxiliary_kind` as the backend projects it. */
export const ORBIT_AUXILIARY_KIND = "AUX_POEORB";

export interface OrbitTaskGroup {
  task: ApplicationTask;
  companions: ApplicationTask[];
}

/** 结合主任务与折入伴随精轨任务推导整组在筛选栏中的代表状态。 */
export function effectiveGroupStatus(group: OrbitTaskGroup): ApplicationTask["status"] {
  const members = [group.task, ...group.companions];
  if (members.some((entry) => entry.status === "running")) return "running";
  if (
    isGacosWebformTask(group.task) &&
    group.companions.length === 0 &&
    (group.task.status === "paused" || group.task.status === "queued")
  ) {
    return "completed";
  }
  if (members.some((entry) => entry.status === "queued")) return "queued";
  if (members.some((entry) => entry.status === "paused")) return "paused";
  if (members.some((entry) => entry.status === "failed")) return "failed";
  return group.task.status;
}

/** 折叠态下内联胶囊显示的伴随精轨摘要状态。 */
export function companionStatusSummaryZh(companions: ApplicationTask[]): string {
  if (companions.some((entry) => entry.status === "running")) return t("downloads.artifacts.companionRunning");
  if (companions.some((entry) => entry.status === "queued")) return t("downloads.artifacts.companionQueued");
  if (companions.some((entry) => entry.status === "paused")) return t("downloads.artifacts.companionPaused");
  if (companions.some((entry) => entry.status === "failed")) return t("downloads.artifacts.companionFailed");
  if (companions.every((entry) => entry.status === "completed")) return t("downloads.artifacts.companionCompleted");
  if (companions.every((entry) => entry.status === "cancelled")) return t("downloads.artifacts.companionCancelled");
  return t("downloads.artifacts.companionPending");
}

/** 分组只使用 S4 投影身份；筛选和队列/历史分区在调用前完成。 */
export function groupOrbitCompanionTasks(tasks: ApplicationTask[]): OrbitTaskGroup[] {
  const rows: OrbitTaskGroup[] = [];
  const byTaskId = new Map<string, OrbitTaskGroup>();
  for (const task of tasks) {
    if (task.auxiliary_kind === ORBIT_AUXILIARY_KIND && task.source_task_id) continue;
    const row = { task, companions: [] };
    rows.push(row);
    byTaskId.set(task.task_id, row);
  }
  for (const task of tasks) {
    if (task.auxiliary_kind !== ORBIT_AUXILIARY_KIND || !task.source_task_id) continue;
    const source = byTaskId.get(task.source_task_id);
    if (source) source.companions.push(task);
    else rows.push({ task, companions: [] });
  }
  return rows;
}

export function isOrbitCompanion(task: ApplicationTask): boolean {
  return task.auxiliary_kind === ORBIT_AUXILIARY_KIND && Boolean(task.source_task_id?.trim());
}

function isPoeorb(item: TaskSceneItem): boolean {
  return item.orbit_recheck?.orbit_type === "POEORB";
}

function extractEofFileName(pathOrName: string | null | undefined): string | null {
  if (!pathOrName) return null;
  const trimmed = pathOrName.trim();
  if (!trimmed) return null;
  const parts = trimmed.split(/[\\/]/);
  const base = parts[parts.length - 1]?.trim() ?? "";
  return base.toUpperCase().endsWith(".EOF") ? base : null;
}

/**
 * S7: 判断轨道条目当前已解析/落盘的轨道产品类型（默认 POEORB，显式使用 RESORB 时返回 RESORB）。
 */
export function resolvedOrbitType(
  item: Pick<TaskSceneItem, "resolved_orbit_type" | "destination" | "orbit_recheck">
): "POEORB" | "RESORB" {
  if (item.resolved_orbit_type === "RESORB") return "RESORB";
  if (item.resolved_orbit_type === "POEORB") return "POEORB";
  const dest = item.destination ?? "";
  const expected = item.orbit_recheck?.expected_file_name ?? "";
  if (
    dest.includes("_AUX_RESORB_") ||
    dest.includes("/AUX_RESORB/") ||
    dest.includes("\\AUX_RESORB\\") ||
    expected.includes("_AUX_RESORB_")
  ) {
    return "RESORB";
  }
  return "POEORB";
}

export interface OrbitRowPresentation {
  label: string;
  badgeClass: string;
}

export function orbitRowStatus(item: TaskSceneItem): OrbitRowPresentation {
  const recheck = isPoeorb(item) ? item.orbit_recheck : null;
  if (item.status === "completed") {
    if (resolvedOrbitType(item) === "RESORB") {
      return {
        label: t("downloads.orbit.downloadedResorb"),
        badgeClass: "bg-sp-warning-bg text-sp-warning-text",
      };
    }
    return { label: t("downloads.orbit.downloaded"), badgeClass: "bg-sp-success-bg text-sp-success-text" };
  }
  if (item.status === "running") {
    return { label: t("downloads.orbit.downloading"), badgeClass: "bg-sp-selected text-sp-accent-strong" };
  }
  if (item.status === "paused") {
    return { label: t("downloads.orbit.paused"), badgeClass: "bg-sp-selected text-sp-text-secondary" };
  }
  if (item.status === "cancelled" || recheck?.state === "cancelled") {
    return { label: t("downloads.orbit.cancelled"), badgeClass: "bg-sp-selected text-sp-text-muted" };
  }
  if (item.status === "failed" || recheck?.state === "exhausted") {
    return { label: t("downloads.orbit.failed"), badgeClass: "bg-sp-danger-bg text-sp-danger-text" };
  }
  if (item.status === "interrupted") {
    return { label: t("downloads.orbit.interrupted"), badgeClass: "bg-sp-selected text-sp-text-secondary" };
  }
  if (recheck?.state === "waiting") {
    return { label: t("downloads.orbit.waiting"), badgeClass: "bg-sp-selected text-sp-text-secondary" };
  }
  return { label: t("downloads.orbit.pendingCheck"), badgeClass: "bg-sp-selected text-sp-text-secondary" };
}

export function formatOrbitCheckTime(item: TaskSceneItem): string | null {
  const recheck = isPoeorb(item) ? item.orbit_recheck : null;
  if (!recheck || recheck.state !== "waiting" || !recheck.next_check_at || item.status !== "queued") {
    return null;
  }
  const date = new Date(recheck.next_check_at);
  if (!Number.isFinite(date.getTime())) return null;
  const pad = (value: number) => String(value).padStart(2, "0");
  return t("downloads.orbit.checkTime", { value: `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}` });
}

export function isOrbitPublicationLate(recheck: OrbitRecheckInfo | null | undefined, now = Date.now()): boolean {
  if (recheck?.orbit_type !== "POEORB" || recheck.state !== "waiting") return false;
  const deadline = Date.parse(recheck.deadline_at);
  return Number.isFinite(deadline) && now >= deadline - 35 * 24 * 60 * 60 * 1000;
}

export function orbitFileInfoLines(item: TaskSceneItem): string[] {
  const recheck = isPoeorb(item) ? item.orbit_recheck : null;
  const fileName = recheck?.expected_file_name?.trim() || extractEofFileName(item.destination);
  const lines = fileName ? [fileName] : [item.scene_id, t("downloads.orbit.fileNameAfterRelease")];
  if (recheck?.state === "exhausted") {
    lines.push(t("downloads.orbit.exhausted"));
  } else if (item.status === "failed" && item.failure) {
    lines.push(preferCodeExplanation(item.failure.reason_zh, item.failure.code) || formatErrorWithCode(item.failure.code));
  }
  return lines;
}

export type OrbitRowAction = "check" | "cancel" | "use_resorb" | "upgrade_poeorb";
export interface OrbitActionAvailability {
  sourcePaused: boolean;
  companionPaused: boolean;
  revisionChanged: boolean;
  inFlight: boolean;
  loadFailed: boolean;
}

/** 这里只派生附属行操作；整任务仍由 getAvailableTaskActions 决定。 */
export function getAvailableOrbitRowActions(
  item: TaskSceneItem,
  availability: OrbitActionAvailability,
): Array<{ kind: OrbitRowAction; disabled: boolean }> {
  const recheck = isPoeorb(item) ? item.orbit_recheck : null;
  const transferTerminal = ["completed", "running", "paused", "cancelled", "interrupted"].includes(item.status);
  const isWaiting = item.status === "queued" && recheck?.state === "waiting";
  const isExhausted = !transferTerminal && recheck?.state === "exhausted";
  const isUnpublishedFailure =
    item.status === "failed" &&
    (recheck !== null || item.failure?.code === "ORB002" || item.failure?.code === "ORB003");
  const canCheck = item.status === "failed" || isExhausted || isWaiting;
  const canUseResorb = isWaiting || isExhausted || isUnpublishedFailure;
  const canUpgradePoeorb = item.status === "completed" && resolvedOrbitType(item) === "RESORB";
  const canCancel = (item.status === "queued" && recheck?.state !== "exhausted" && recheck?.state !== "cancelled") || ((item.status === "running" || item.status === "paused") && recheck?.state !== "exhausted");
  const actions: Array<{ kind: OrbitRowAction; disabled: boolean }> = [];
  const globallyDisabled = availability.revisionChanged || availability.inFlight || availability.loadFailed;
  const checkOrResorbDisabled =
    globallyDisabled || availability.sourcePaused || availability.companionPaused;
  if (canCheck) {
    actions.push({
      kind: "check",
      disabled: checkOrResorbDisabled,
    });
  }
  if (canUseResorb) {
    actions.push({
      kind: "use_resorb",
      disabled: checkOrResorbDisabled,
    });
  }
  if (canUpgradePoeorb) {
    actions.push({
      kind: "upgrade_poeorb",
      disabled: checkOrResorbDisabled,
    });
  }
  if (canCancel) {
    actions.push({ kind: "cancel", disabled: globallyDisabled });
  }
  return actions;
}
