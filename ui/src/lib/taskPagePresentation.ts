import type { TaskItemLocalReuseProjection, TaskPageProjection } from "./coreRestoreProjection.ts";
import type { DesktopJsonObject } from "./desktopShellBridge.ts";

const TERMINAL_ITEM_STATES = new Set([
  "completed",
  "failed",
  "skipped",
  "cancelled",
  "interrupted",
]);

export type TaskPagePresentationItem = {
  itemId: string;
  displayName: string;
  status: string;
  bytesCompleted: number;
  totalBytes: number | null;
  sourceTotalBytes: number | null;
  percentage: number | null;
  bytesPerSecond: number;
  destination: string | null;
  failure: {
    code: string;
    reasonZh: string;
    retryable: boolean;
    stage: string;
    transferredBytes: number;
    attempt: number;
  } | null;
  attempt: number;
  retryable: boolean;
  terminal: boolean;
  metadata: DesktopJsonObject;
  /** Downloader D3: the item's local reuse decision; absent for none. */
  localReuse?: TaskItemLocalReuseProjection | null;
};

export type TaskPagePresentation = {
  taskId: string;
  revision: number;
  pageIndex: number;
  pageSize: number;
  totalCount: number;
  sourceTotalBytes: number | null;
  networkBytesReceived: number | null;
  items: TaskPagePresentationItem[];
};

export function taskPercent(done: number, total: number | null): number | null {
  return total !== null && total > 0
    ? Math.min(100, Math.max(0, (done / total) * 100))
    : null;
}

export function etaSeconds(done: number, total: number | null, rate: number): number | null {
  return total !== null && total >= done && rate > 0 ? (total - done) / rate : null;
}

export function taskOverviewItems(page: TaskPagePresentation | null): TaskPagePresentationItem[] {
  return page ? [...page.items] : [];
}

export function taskItemHasCurrentFailure(item: TaskPagePresentationItem): boolean {
  return item.failure !== null
    && ["failed", "interrupted"].includes(item.status.toLowerCase());
}

export function taskItemHasCurrentFailureObservation(item: TaskPagePresentationItem): boolean {
  return item.failure !== null
    && ["failed", "interrupted", "cancelled", "skipped"].includes(item.status.toLowerCase());
}

// Local validation results whose codes would otherwise match a provider stage.
const LOCAL_VALIDATION_FAILURE_CODES = new Set([
  // A refused one-time budget upgrade: checked locally, no network request sent.
  "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
]);

export function taskFailureStage(code: string): string {
  const normalized = code.toUpperCase();
  if (LOCAL_VALIDATION_FAILURE_CODES.has(normalized)) {
    return "本地校验";
  }
  // The lookahead keeps scientific *_AUTHORITY_* codes out of the login stage.
  if (normalized === "DL004" || /LOGIN|SESSION|CREDENTIAL|AUTH(?!ORIT)/.test(normalized)) {
    return "登录/会话";
  }
  if (normalized.includes("PREPARE") || normalized.includes("SOURCE") || normalized.includes("METADATA")) {
    return "准备元数据";
  }
  if (normalized.includes("PLAN") || normalized.includes("MAPPING") || normalized.includes("GRAPH") || normalized.includes("INTENT")) {
    return "规划 AOI 子集";
  }
  if (normalized.includes("RANGE") || normalized.includes("TRANSPORT") || normalized.includes("TRANSFER") || normalized.includes("PARTIAL") || normalized.includes("BUDGET")) {
    return "下载数据范围";
  }
  if (normalized.includes("WRITER") || normalized.includes("ARTIFACT")) {
    return "生成 HDF5";
  }
  if (normalized.includes("OUTPUT")) {
    return "输出";
  }
  if (normalized.includes("VALID") || normalized.includes("SCIENCE") || normalized.includes("CHECK")) {
    return "校验";
  }
  return "未知阶段";
}

export function projectTaskPagePresentation(page: TaskPageProjection): TaskPagePresentation {
  return {
    taskId: page.task_id,
    revision: page.revision,
    pageIndex: page.page_index,
    pageSize: page.page_size,
    totalCount: page.total_count,
    sourceTotalBytes: page.source_total_bytes,
    networkBytesReceived: page.network_bytes_received,
    items: page.items.map((entry) => {
      const terminal = TERMINAL_ITEM_STATES.has(entry.status);
      const percentage = entry.total_bytes === null || entry.total_bytes <= 0
        ? null
        : Math.min(100, Math.round((entry.bytes_completed / entry.total_bytes) * 100));
      return {
        itemId: entry.item.item_id,
        displayName: entry.item.display_name,
        status: entry.status,
        bytesCompleted: entry.bytes_completed,
        totalBytes: entry.total_bytes,
        sourceTotalBytes: entry.source_total_bytes,
        percentage,
        bytesPerSecond: terminal ? 0 : entry.bytes_per_second,
        destination: entry.destination,
        failure: entry.failure
          ? {
              code: entry.failure.code,
              reasonZh: entry.failure.reason_zh,
              retryable: entry.failure.retryable,
              stage: taskFailureStage(entry.failure.code),
              transferredBytes: entry.bytes_completed,
              attempt: entry.attempt,
            }
          : null,
        attempt: entry.attempt,
        retryable: entry.retryable,
        terminal,
        metadata: structuredClone(entry.item.metadata),
        localReuse: entry.local_reuse,
      };
    }),
  };
}
