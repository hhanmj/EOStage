/**
 * 结果列表纯逻辑模块（W5-3b）
 * 包含排序、选择状态、勾选汇总、分页、失败摘要、行内进度及错误文本解析。
 */

import type { SceneRow } from "../../lib/bridge.ts";
import { t } from "../../i18n/index.ts";
import type { RowTaskState, ApplicationTask, TaskFailureSummary } from "./types.ts";
import { formatBytes, extractCloudCover } from "./rowProjection.ts";
import { preferCodeExplanation, explainActionError, explainDownloadCode } from "./errorExplanations.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";

export type SortKey = "date" | "relativeOrbit" | "frame" | "size" | "cloud" | "source";
export type SortDir = "asc" | "desc";

export interface SortState {
  key: SortKey;
  dir: SortDir;
}

export const DEFAULT_SORT_STATE: SortState = {
  key: "date",
  dir: "desc",
};

export interface SortKeyOptions {
  hasOpticalKeys?: boolean;
  hasSarKeys?: boolean;
  hasSourceKey?: boolean;
}

/**
 * 根据数据源形态（SAR vs 光学 vs 多源混合）返回对应的可用排序项（W5-3a-F2）
 */
export function sortKeysFor(
  kind: "sar" | "optical" | "mixed" | null,
  hasCloudKey = true,
  options: SortKeyOptions = {}
): Array<{ key: SortKey; label: string }> {
  if (kind === "mixed") {
    const keys: Array<{ key: SortKey; label: string }> = [
      { key: "date", label: t("mapWorkbench.results.date") },
    ];
    if (options.hasOpticalKeys && hasCloudKey) {
      keys.push({ key: "cloud", label: t("mapWorkbench.results.cloud") });
    }
    if (options.hasSarKeys) {
      keys.push({ key: "relativeOrbit", label: "Path" });
      keys.push({ key: "frame", label: "Frame" });
    }
    if (options.hasSourceKey) {
      keys.push({ key: "source", label: t("agent.multiBatch.sortBySource") });
    }
    keys.push({ key: "size", label: t("mapWorkbench.results.size") });
    return keys;
  }

  if (kind === "optical") {
    const keys: Array<{ key: SortKey; label: string }> = [
      { key: "date", label: t("mapWorkbench.results.date") },
    ];
    if (hasCloudKey) {
      keys.push({ key: "cloud", label: t("mapWorkbench.results.cloud") });
    }
    keys.push({ key: "size", label: t("mapWorkbench.results.size") });
    return keys;
  }

  return [
    { key: "date", label: t("mapWorkbench.results.date") },
    { key: "relativeOrbit", label: "Path" },
    { key: "frame", label: "Frame" },
    { key: "size", label: t("mapWorkbench.results.size") },
  ];
}

/**
 * 结果列表前端稳定排序
 * 缺失/无效值在 asc/desc 两个方向下均始终垫底排在最后；
 * 同分场景按照 scene_id 升序决胜（稳定、确定性）。
 */
export function sortScenes(
  scenes: SceneRow[],
  key: SortKey,
  dir: SortDir,
  resolveSourceLabel?: (scene: SceneRow) => string
): SceneRow[] {
  const result = [...scenes];

  result.sort((a, b) => {
    let aVal: number | string | null = null;
    let bVal: number | string | null = null;
    let aValid = false;
    let bValid = false;

    if (key === "date") {
      const aDate = a.acquisition_datetime?.trim();
      const bDate = b.acquisition_datetime?.trim();
      if (aDate) {
        aVal = aDate;
        aValid = true;
      }
      if (bDate) {
        bVal = bDate;
        bValid = true;
      }
    } else if (key === "source") {
      const aSrc = (resolveSourceLabel?.(a) ?? (a.platform || a.provider || "")).trim().toLowerCase();
      const bSrc = (resolveSourceLabel?.(b) ?? (b.platform || b.provider || "")).trim().toLowerCase();
      if (aSrc) {
        aVal = aSrc;
        aValid = true;
      }
      if (bSrc) {
        bVal = bSrc;
        bValid = true;
      }
    } else if (key === "relativeOrbit") {
      const aNum = a.relative_orbit;
      const bNum = b.relative_orbit;
      if (typeof aNum === "number" && !Number.isNaN(aNum) && Number.isFinite(aNum)) {
        aVal = aNum;
        aValid = true;
      }
      if (typeof bNum === "number" && !Number.isNaN(bNum) && Number.isFinite(bNum)) {
        bVal = bNum;
        bValid = true;
      }
    } else if (key === "frame") {
      const aNum = a.frame;
      const bNum = b.frame;
      if (typeof aNum === "number" && !Number.isNaN(aNum) && Number.isFinite(aNum)) {
        aVal = aNum;
        aValid = true;
      }
      if (typeof bNum === "number" && !Number.isNaN(bNum) && Number.isFinite(bNum)) {
        bVal = bNum;
        bValid = true;
      }
    } else if (key === "size") {
      const aNum = a.file_size_remote;
      const bNum = b.file_size_remote;
      if (typeof aNum === "number" && !Number.isNaN(aNum) && Number.isFinite(aNum) && aNum >= 0) {
        aVal = aNum;
        aValid = true;
      }
      if (typeof bNum === "number" && !Number.isNaN(bNum) && Number.isFinite(bNum) && bNum >= 0) {
        bVal = bNum;
        bValid = true;
      }
    } else if (key === "cloud") {
      const aNum = extractCloudCover(a);
      const bNum = extractCloudCover(b);
      if (typeof aNum === "number" && !Number.isNaN(aNum) && Number.isFinite(aNum)) {
        aVal = aNum;
        aValid = true;
      }
      if (typeof bNum === "number" && !Number.isNaN(bNum) && Number.isFinite(bNum)) {
        bVal = bNum;
        bValid = true;
      }
    }

    // 核心规则：缺失/无效值在 asc 与 desc 两个方向均始终排在有值数据之后
    if (aValid && !bValid) return -1;
    if (!aValid && bValid) return 1;
    if (!aValid && !bValid) {
      return a.scene_id < b.scene_id ? -1 : (a.scene_id > b.scene_id ? 1 : 0);
    }

    // 两者均有效
    if (aVal! < bVal!) {
      return dir === "asc" ? -1 : 1;
    }
    if (aVal! > bVal!) {
      return dir === "asc" ? 1 : -1;
    }

    // 同分平局按 scene_id 升序
    return a.scene_id < b.scene_id ? -1 : (a.scene_id > b.scene_id ? 1 : 0);
  });

  return result;
}

/**
 * 排序状态机转移
 * 相同 key 翻转方向；切换新 key 时 date/size 默认 desc，relativeOrbit/frame 默认 asc。
 */
export function nextSort(
  state: SortState,
  key: SortKey
): SortState {
  if (state.key === key) {
    return {
      key,
      dir: state.dir === "asc" ? "desc" : "asc",
    };
  }

  const dir: SortDir = (key === "date" || key === "size") ? "desc" : "asc";
  return { key, dir };
}

/**
 * 判断场景行是否可勾选
 * 运行中、排队中、暂停、完成、中断不可勾选；无状态、失败或取消可勾选。
 */
export function isRowSelectable(
  rowState?: RowTaskState | null
): boolean {
  if (!rowState || !rowState.task_status) {
    return true;
  }
  const status = rowState.task_status;
  if (
    status === "queued" ||
    status === "running" ||
    status === "paused" ||
    status === "completed" ||
    status === "interrupted"
  ) {
    return false;
  }
  return true;
}

/**
 * 提取所有可勾选的场景 ID
 */
export function selectableIds(
  scenes: SceneRow[],
  rowStates: Map<string, RowTaskState>
): string[] {
  return scenes
    .filter((s) => isRowSelectable(rowStates.get(s.scene_id)))
    .map((s) => s.scene_id);
}

/**
 * 全选切换
 * 若所有可选 id 均已勾选，则清空；否则勾选全部可选 id。
 */
export function toggleAll(
  checked: Set<string>,
  selectable: string[]
): Set<string> {
  if (selectable.length === 0) {
    return new Set();
  }
  const allSelected = selectable.every((id) => checked.has(id));
  if (allSelected) {
    return new Set();
  }
  return new Set(selectable);
}

/**
 * 计算表头全选复选框三态
 * 仅以 selectable 为基准
 */
export function headerCheckState(
  checked: Set<string>,
  selectable: string[]
): "none" | "some" | "all" {
  if (selectable.length === 0) {
    return "none";
  }
  const selectedCount = selectable.filter((id) => checked.has(id)).length;
  if (selectedCount === 0) {
    return "none";
  }
  if (selectedCount === selectable.length) {
    return "all";
  }
  return "some";
}

export interface CheckedSummaryResult {
  count: number;
  knownBytes: number;
  unknownCount: number;
  summaryText: string;
}

/**
 * 计算勾选汇总统计与展示文本
 */
export function checkedSummary(
  scenes: SceneRow[],
  checked: Set<string>
): CheckedSummaryResult {
  const checkedScenes = scenes.filter((s) => checked.has(s.scene_id));
  const count = checkedScenes.length;

  if (count === 0) {
    return {
      count: 0,
      knownBytes: 0,
      unknownCount: 0,
      summaryText: t("mapWorkbench.results.checkedNone"),
    };
  }

  let knownBytes = 0;
  let unknownCount = 0;

  for (const s of checkedScenes) {
    const size = s.file_size_remote;
    if (typeof size === "number" && !Number.isNaN(size) && Number.isFinite(size) && size >= 0) {
      knownBytes += size;
    } else {
      unknownCount += 1;
    }
  }

  let summaryText = "";
  if (unknownCount === 0) {
    summaryText = t("mapWorkbench.results.checkedKnown", { checked: count, size: formatBytes(knownBytes) });
  } else if (knownBytes > 0) {
    summaryText = t("mapWorkbench.results.checkedPartialUnknown", { checked: count, size: formatBytes(knownBytes), unknown: unknownCount });
  } else {
    summaryText = t("mapWorkbench.results.checkedUnknown", { checked: count });
  }

  return {
    count,
    knownBytes,
    unknownCount,
    summaryText,
  };
}

export type ResultSelectionScope = "current_page" | "loaded" | "all_matching";

export interface AllMatchSelectionState {
  active: boolean;
  excludedIds: Set<string>;
}

/**
 * 切换本页可勾选集合：若本页所有可选行均已勾选，则仅取消本页可选行；否则勾选本页所有可选行。
 */
export function togglePageSelection(
  checked: Set<string>,
  pageSelectable: string[]
): Set<string> {
  const next = new Set(checked);
  if (pageSelectable.length === 0) {
    return next;
  }
  const allSelected = pageSelectable.every((id) => next.has(id));
  for (const id of pageSelectable) {
    if (allSelected) next.delete(id);
    else next.add(id);
  }
  return next;
}

/**
 * 在全部匹配（all_matching）模式下，判断单行是否处于勾选态。
 */
export function isRowCheckedInScope(
  sceneId: string,
  rowState: RowTaskState | undefined | null,
  checkedSceneIds: Set<string>,
  allMatch?: AllMatchSelectionState | null
): boolean {
  if (allMatch?.active) {
    return isRowSelectable(rowState) && !allMatch.excludedIds.has(sceneId);
  }
  return checkedSceneIds.has(sceneId);
}

/**
 * 在全部匹配（all_matching）模式下，切换单行的排除状态（excluded_item_ids 增量）。
 */
export function toggleRowInAllMatch(
  excludedIds: Set<string>,
  sceneId: string
): Set<string> {
  const next = new Set(excludedIds);
  if (next.has(sceneId)) next.delete(sceneId);
  else next.add(sceneId);
  return next;
}

/**
 * 在全部匹配（all_matching）模式下，切换本页可选行的排除状态。
 */
export function togglePageInAllMatch(
  excludedIds: Set<string>,
  pageSelectable: string[]
): Set<string> {
  const next = new Set(excludedIds);
  if (pageSelectable.length === 0) return next;
  const allIncluded = pageSelectable.every((id) => !next.has(id));
  for (const id of pageSelectable) {
    if (allIncluded) next.add(id);
    else next.delete(id);
  }
  return next;
}

/**
 * 在全部匹配（all_matching）模式下，计算当前已加载行的三态勾选状态（表头复选框与本页按钮共用）。
 */
export function checkStateInScope(
  selectable: string[],
  checkedSceneIds: Set<string>,
  allMatch?: AllMatchSelectionState | null
): "none" | "some" | "all" {
  if (selectable.length === 0) return "none";
  if (allMatch?.active) {
    const includedCount = selectable.filter((id) => !allMatch.excludedIds.has(id)).length;
    if (includedCount === 0) return "none";
    if (includedCount === selectable.length) return "all";
    return "some";
  }
  return headerCheckState(checkedSceneIds, selectable);
}

/**
 * 全部匹配模式下的顶部汇总统计：以批次匹配总数扣减已加载不可选行与排除行。
 */
export function allMatchSummary(
  totalMatched: number,
  loadedScenes: SceneRow[],
  rowStates: Map<string, RowTaskState>,
  excludedIds: Set<string>
): CheckedSummaryResult {
  const nonSelectableLoadedCount = loadedScenes.filter(
    (s) => !isRowSelectable(rowStates.get(s.scene_id))
  ).length;
  const excludedSelectableCount = Array.from(excludedIds).filter((id) =>
    isRowSelectable(rowStates.get(id))
  ).length;
  const count = Math.max(0, totalMatched - nonSelectableLoadedCount - excludedSelectableCount);
  if (count === 0) {
    return {
      count: 0,
      knownBytes: 0,
      unknownCount: 0,
      summaryText: t("mapWorkbench.results.checkedNone"),
    };
  }
  const summaryText =
    excludedSelectableCount > 0
      ? t("mapWorkbench.results.allMatchingCheckedExcluded", {
          checked: count,
          excluded: excludedSelectableCount,
        })
      : t("mapWorkbench.results.allMatchingChecked", { checked: count });
  return {
    count,
    knownBytes: 0,
    unknownCount: count,
    summaryText,
  };
}

export interface PaginateResult<T> {
  items: T[];
  page: number;
  pageCount: number;
}

/**
 * 列表纯前端分页
 */
export function paginate<T>(
  list: T[],
  page: number,
  pageSize: number = RESULT_PAGE_SIZE
): PaginateResult<T> {
  const size = Math.max(1, pageSize);
  const pageCount = Math.max(1, Math.ceil(list.length / size));
  const clampedPage = Math.min(Math.max(1, Math.floor(page) || 1), pageCount);
  const start = (clampedPage - 1) * size;
  const items = list.slice(start, start + size);

  return {
    items,
    page: clampedPage,
    pageCount,
  };
}

/**
 * Strip internal diagnostic payloads such as `INTEGRITY_DIAGNOSTIC {...}` from
 * user-facing failure banners/cards while preserving clean natural language.
 */
export function stripInternalDiagnosticSuffix(
  text: string | null | undefined
): string {
  if (!text || typeof text !== "string") {
    return "";
  }
  const stripped = text
    .replace(/\s*[\(（]?\s*INTEGRITY_DIAGNOSTIC\b[\s\S]*$/i, "")
    .replace(/[;；:\s]+$/, "")
    .trim();
  return stripped;
}

/**
 * 失败首句截取
 * 截取至首个句号（含）或首个换行符（不含），上限 120 字符附加省略号。
 */
export function failureFirstSentence(
  text: string | null | undefined
): string {
  const cleaned = stripInternalDiagnosticSuffix(text);
  if (!cleaned) {
    return "";
  }
  const trimmedText = cleaned;

  const dotIndex = trimmedText.indexOf("。");
  const newlineIndex = trimmedText.search(/[\r\n]/);

  let sentence = trimmedText;
  if (dotIndex !== -1 && newlineIndex !== -1) {
    if (dotIndex < newlineIndex) {
      sentence = trimmedText.slice(0, dotIndex + 1);
    } else {
      sentence = trimmedText.slice(0, newlineIndex);
    }
  } else if (dotIndex !== -1) {
    sentence = trimmedText.slice(0, dotIndex + 1);
  } else if (newlineIndex !== -1) {
    sentence = trimmedText.slice(0, newlineIndex);
  }

  sentence = sentence.trim();
  if (sentence.length > 120) {
    sentence = sentence.slice(0, 120) + "…";
  }
  return sentence;
}

/**
 * 任务失败原因统一呈现
 */
export function failureSummaryText(
  failure: TaskFailureSummary | null | undefined
): string {
  if (!failure) {
    return t("mapWorkbench.results.failureReasonMissing");
  }

  const firstSentence =
    typeof failure.message_zh === "string" && failure.message_zh.trim() !== ""
      ? failureFirstSentence(failure.message_zh)
      : null;
  const firstCode =
    Array.isArray(failure.codes) && failure.codes.length > 0 ? failure.codes[0] : null;
  let baseReason =
    preferCodeExplanation(firstSentence, firstCode) ??
    explainDownloadCode(firstCode) ??
    "";

  if (!baseReason) {
    baseReason = t("mapWorkbench.results.failureReasonMissing");
  }

  if (
    typeof failure.total_items === "number" &&
    Number.isFinite(failure.total_items) &&
    failure.total_items > 1 &&
    typeof failure.failed_items === "number" &&
    Number.isFinite(failure.failed_items)
  ) {
    return t("mapWorkbench.results.multipleFailures", { failed: failure.failed_items, total: failure.total_items, reason: baseReason });
  }

  return baseReason;
}

/**
 * 单景任务传输进度百分比
 * 仅单景、total_bytes > 0 且字节已上报时返回百分比整数，多景任务、字节未上报或无效值严格返回 null。
 */
export function rowProgressPercent(
  task: ApplicationTask | undefined
): number | null {
  if (
    task &&
    task.items_count === 1 &&
    typeof task.total_bytes === "number" &&
    task.total_bytes > 0 &&
    task.bytes_reported !== false &&
    typeof task.done_bytes === "number" &&
    task.done_bytes >= 0
  ) {
    return Math.floor((task.done_bytes / task.total_bytes) * 100);
  }
  return null;
}

export interface TaskOutcomeNotice {
  tone: "danger" | "neutral";
  text: string;
}

export function createTaskOutcomeNotice(
  resultOrError: unknown
): TaskOutcomeNotice | null {
  if (resultOrError === null || resultOrError === undefined) {
    return null;
  }

  if (typeof resultOrError === "object" && "created" in (resultOrError as Record<string, unknown>)) {
    const created = (resultOrError as { created: unknown }).created;
    if (created === false) {
      return {
        tone: "neutral",
        text: t("mapWorkbench.results.existingTask"),
      };
    }
    if (created === true) {
      const rec = resultOrError as Record<string, unknown>;
      const skipped = typeof rec.skipped_count === "number" ? rec.skipped_count : 0;
      const rejected = typeof rec.rejected_count === "number" ? rec.rejected_count : 0;
      const createdCount = typeof rec.created_count === "number" ? rec.created_count : 0;
      if (skipped > 0 || rejected > 0) {
        return {
          tone: "neutral",
          text: t("mapWorkbench.results.allMatchTaskOutcomeSummary", {
            created: createdCount,
            skipped,
            rejected,
          }),
        };
      }
      return null;
    }
  }

  const text = createTaskErrorText(resultOrError);
  return {
    tone: "danger",
    text,
  };
}

/**
 * 创建任务错误消息呈现
 */
export function createTaskErrorText(
  err: unknown
): string {
  if (
    err &&
    typeof err === "object" &&
    "code" in err &&
    (err as { code: unknown }).code === "STORAGE_ROOT_NOT_WRITABLE"
  ) {
    const msgZh = (err as { message_zh?: unknown }).message_zh;
    const text = preferCodeExplanation(
      typeof msgZh === "string" && msgZh.trim() !== "" ? msgZh.trim() : null,
      "STORAGE_ROOT_NOT_WRITABLE",
    );
    if (text) {
      return text;
    }
  }
  // #22 B slice 5: a batch that is still loading admits only rows that arrived.
  const code = err && typeof err === "object" ? (err as { code?: unknown }).code : undefined;
  if (code === "V3_STORE_BATCH_ROWS_NOT_YET_LOADED") {
    return t("mapWorkbench.results.rowsNotArrived");
  }
  if (code === "V3_STORE_BATCH_STILL_LOADING") {
    return t("mapWorkbench.results.selectAllMatchingWaiting");
  }
  if (code === "V3_TASK_BATCH_INCOMPLETE") {
    return t("mapWorkbench.results.allMatchBatchIncomplete");
  }
  if (code === "V3_TASK_OVER_CEILING") {
    return t("mapWorkbench.results.allMatchOverCeiling", { limit: 10000 });
  }
  if (code === "V3_TASK_SELECTION_CHANGED") {
    return t("mapWorkbench.results.allMatchSelectionChanged");
  }
  if (code === "V3_TASK_EXCLUSIONS_TOO_MANY") {
    return t("mapWorkbench.results.allMatchTooManyExclusions", { limit: 250 });
  }
  if (code === "V3_STORE_PARTIAL_ACTIVE_TASK_OVERLAP") {
    return t("mapWorkbench.results.partialActiveTaskOverlap");
  }
  return explainActionError(err, t("mapWorkbench.results.taskCreationFailed"));
}
