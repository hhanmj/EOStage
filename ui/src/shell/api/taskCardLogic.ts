/**
 * 任务卡片尺寸与状态行逻辑
 * 审计 P0：不伪造值；「已知总量」只能来自后端给的 total_bytes / source_total_bytes。
 */

import { t } from "../../i18n/index.ts";
import { lookupSubsetSizeEstimate } from "../../lib/subsetSizeEstimateRegistry.ts";
import type { ApplicationTask, TaskPauseReason } from "./types.ts";
import { formatErrorWithCode } from "./errorExplanations.ts";

/**
 * 格式化任务暂停原因文本行
 * 后端 format_gb 口径见 crates/insar-core/src/storage_guard.rs:82-100
 */
export function pauseReasonLine(reason?: TaskPauseReason | null): string | null {
  if (!reason || reason.kind !== "disk_low") {
    return null;
  }
  const driveLabel = reason.drive && reason.drive.trim() ? reason.drive : t("shared.taskCard.unknownDrive");
  const gb = (reason.free_bytes / (2 ** 30)).toFixed(1);
  return t("shared.taskCard.diskLowPaused", { driveLabel, gb });
}

/**
 * 格式化任务卡片尺寸与吞吐行文本
 * - total_bytes 已知（正数）→ 现状不变（done/total）
 * - total_bytes 未知而 source_total_bytes 已知（正数）且处于终态（cancelled/interrupted/failed/completed）→ 显示「已传输 X · 冻结总量 Y」
 * - 两者都未知 → 保持「大小未知」（若 done_bytes > 0 则「已传输 X · 总量未知」）
 * - 运行中 (running) 或非终态不使用 source_total_bytes 回退
 * - bytes_reported === false（归档历史，字节未上报）→ 不显示已传字节：有总量时「总量 Y」，否则「大小未知」
 */
export function taskCardSizeLine(
  task: Pick<ApplicationTask, "status" | "done_bytes" | "total_bytes" | "source_total_bytes" | "bytes_reported"> & {
    task_id?: string | null;
    scene_ids?: readonly string[] | null;
    estimated_bytes?: number | null;
  },
  formatBytesFn: (bytes?: number | null) => string,
  explicitEstimatedBytes?: number | null,
): string {
  const hasTotalBytes = typeof task.total_bytes === "number" && task.total_bytes > 0;
  if (task.bytes_reported === false) {
    return hasTotalBytes
      ? t("shared.taskCard.totalBytes", { total: formatBytesFn(task.total_bytes!) })
      : t("shared.taskCard.unknownSize");
  }
  if (hasTotalBytes) {
    return `${formatBytesFn(task.done_bytes)} / ${formatBytesFn(task.total_bytes!)}`;
  }

  // 终态回退至冻结清单总量 (source_total_bytes)
  const isTerminal =
    task.status === "cancelled" ||
    task.status === "interrupted" ||
    task.status === "failed" ||
    task.status === "completed";

  const hasSourceTotal = typeof task.source_total_bytes === "number" && task.source_total_bytes > 0;

  if (isTerminal && hasSourceTotal) {
    return t("shared.taskCard.transferredWithFrozenTotal", {
      done: formatBytesFn(task.done_bytes),
      total: formatBytesFn(task.source_total_bytes!),
    });
  }

  // 裁剪任务在服务端尚无 total_bytes 时回退显示客户端按 AOI 占比估算值（「预计 ~X」），
  // 完成后自动切换为真实磁盘字节数
  const estimatedBytes =
    typeof explicitEstimatedBytes === "number" && explicitEstimatedBytes > 0
      ? explicitEstimatedBytes
      : typeof task.estimated_bytes === "number" && task.estimated_bytes > 0
        ? task.estimated_bytes
        : lookupSubsetSizeEstimate(task.task_id, task.scene_ids);

  if (!isTerminal && typeof estimatedBytes === "number" && estimatedBytes > 0) {
    if (task.done_bytes > 0) {
      return t("shared.taskCard.transferredWithEstimatedTotal", {
        done: formatBytesFn(task.done_bytes),
        total: formatBytesFn(estimatedBytes),
      });
    }
    return t("shared.taskCard.estimatedTotal", {
      total: formatBytesFn(estimatedBytes),
    });
  }

  // 运行中或无冻结总量
  if (task.status === "completed") {
    return formatBytesFn(task.done_bytes);
  }
  if (task.done_bytes > 0) {
    return t("shared.taskCard.transferredWithUnknownTotal", {
      done: formatBytesFn(task.done_bytes),
    });
  }
  return t("shared.taskCard.unknownSize");
}

export interface FailureReasonLineResult {
  primary: string;
  secondary: string | null;
}

/**
 * 失败原因行展示逻辑（预留失败原因行：已知码中文映射，未知码原样，无码时显示「失败原因未返回」）
 */
export function failureReasonLine(
  taskOrError?: Pick<ApplicationTask, "error"> | ApplicationTask["error"]
): FailureReasonLineResult {
  const error =
    taskOrError && typeof taskOrError === "object" && "error" in taskOrError
      ? taskOrError.error
      : taskOrError;

  if (typeof error === "string") {
    const trimmed = error.trim();
    if (trimmed) {
      return { primary: trimmed, secondary: null };
    }
    return { primary: t("shared.taskCard.failureReasonNotReturned"), secondary: null };
  }

  if (error && typeof error === "object") {
    const rawCode = (error as { code?: unknown }).code;
    const rawMessageZh = (error as { message_zh?: unknown }).message_zh;
    const code = typeof rawCode === "string" ? rawCode.trim() : "";
    const messageZh = typeof rawMessageZh === "string" ? rawMessageZh.trim() : "";

    if (code) {
      const primary = formatErrorWithCode(code);
      const secondary = messageZh && messageZh !== primary ? messageZh : null;
      return { primary, secondary };
    }

    if (messageZh) {
      return { primary: messageZh, secondary: null };
    }
  }

  return { primary: t("shared.taskCard.failureReasonNotReturned"), secondary: null };
}
