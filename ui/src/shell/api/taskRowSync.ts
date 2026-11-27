import type { RowTaskState } from "./types.ts";
import { mergeRowStates as defaultMergeRowStates } from "./rowStateSync.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";

export interface TaskSnapshotDigestItem {
  task_id: string;
  status?: string | null;
  revision?: number | null;
}

export interface TaskRowSyncDependencies {
  getActiveBatch: () => { batchId: string; revision: number } | null;
  getBatchPage: (
    batchId: string,
    revision: number,
    page?: number,
    pageSize?: number
  ) => Promise<{ scenes?: any[]; rows: RowTaskState[] }>;
  setRowStates: (updater: (prev: Map<string, RowTaskState>) => Map<string, RowTaskState>) => void;
  mergeRowStates?: (
    prev: Map<string, RowTaskState>,
    rows: RowTaskState[]
  ) => { next: Map<string, RowTaskState>; changed: boolean };
  batchPageInFlightRef?: { current: boolean };
  isMounted?: () => boolean;
}

export interface TaskRowSyncer {
  syncOnTaskSnapshot: (
    tasks: TaskSnapshotDigestItem[],
    options: { drawerOpen: boolean }
  ) => Promise<boolean>;
  getLastSignature: () => string | null;
  reset: () => void;
}

/**
 * 按 (task_id, status, revision) 计算快照签名
 * 排序以保证与任务列表顺序无关
 */
export function computeTasksSignature(tasks: TaskSnapshotDigestItem[]): string {
  if (!tasks || tasks.length === 0) return "";
  return tasks
    .map((t) => `${t.task_id}:${t.status ?? ""}:${t.revision ?? 0}`)
    .sort()
    .join("|");
}

/**
 * 创建结果行任务状态同步协调器
 *
 * 核心逻辑与生命周期解耦（项 1）：
 * 1. 任务总览订阅维持在 Shell 顶层（taskOverviewSource），有订阅者期间持续轮询（可见 1 s / 隐藏 10 s）；
 * 2. 抽屉打开时：维持既有行为，每次快照都持续刷新行状态；
 * 3. 抽屉关闭时：按 (task_id, status, revision) 签名比对，只要任务状态或版本发生迁移（如 running -> paused -> cancelled），
 *    即便抽屉关闭也立即拉取一次批次页并经 mergeRowStates 落入行状态 setter。
 */
export function createTaskRowSyncer(deps: TaskRowSyncDependencies): TaskRowSyncer {
  let lastSignature: string | null = null;
  let inFlight = false;
  const mergeFn = deps.mergeRowStates || defaultMergeRowStates;

  const setInFlight = (val: boolean) => {
    inFlight = val;
    if (deps.batchPageInFlightRef) {
      deps.batchPageInFlightRef.current = val;
    }
  };

  return {
    syncOnTaskSnapshot: async (tasks: TaskSnapshotDigestItem[], { drawerOpen }) => {
      const activeBatch = deps.getActiveBatch();
      if (!activeBatch || inFlight) {
        return false;
      }

      const nextSignature = computeTasksSignature(tasks);
      const signatureChanged = lastSignature === null || lastSignature !== nextSignature;
      const shouldFetch = drawerOpen || signatureChanged;

      if (!shouldFetch) {
        return false;
      }

      setInFlight(true);
      try {
        const { batchId, revision } = activeBatch;
        const page = await deps.getBatchPage(batchId, revision, 0, RESULT_PAGE_SIZE);
        if (deps.isMounted && !deps.isMounted()) {
          return false;
        }
        if (page?.rows) {
          deps.setRowStates((prev) => {
            const { next, changed } = mergeFn(prev, page.rows);
            return changed ? next : prev;
          });
        }
        lastSignature = nextSignature;
        return true;
      } catch {
        return false;
      } finally {
        setInFlight(false);
      }
    },
    getLastSignature: () => lastSignature,
    reset: () => {
      lastSignature = null;
      setInFlight(false);
    },
  };
}
