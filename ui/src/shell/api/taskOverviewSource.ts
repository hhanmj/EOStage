import { t } from "../../i18n/index.ts";
import type { ApplicationTask } from "./types.ts";
import { defaultTimerDependencies, type PollerTimerDependencies } from "./poller.ts";
import { computeTasksSignature } from "./taskRowSync.ts";

export type TaskOverviewListener = (tasks: ApplicationTask[]) => void;

export interface TaskOverviewVisibilityDependencies {
  isHidden: () => boolean;
  onVisibilityChange: (handler: () => void) => () => void;
}

export const defaultVisibilityDependencies: TaskOverviewVisibilityDependencies = {
  isHidden: () =>
    typeof document !== "undefined" && document.visibilityState === "hidden",
  onVisibilityChange: (handler) => {
    if (
      typeof document === "undefined" ||
      typeof document.addEventListener !== "function"
    ) {
      return () => {};
    }
    document.addEventListener("visibilitychange", handler);
    return () => {
      if (
        typeof document !== "undefined" &&
        typeof document.removeEventListener === "function"
      ) {
        document.removeEventListener("visibilitychange", handler);
      }
    };
  },
};

export interface TaskOverviewSourceOptions {
  fetchTasks: () => Promise<ApplicationTask[]>;
  pollIntervalMs?: number;
  hiddenPollIntervalMs?: number;
  timers?: PollerTimerDependencies;
  visibility?: TaskOverviewVisibilityDependencies;
}

export interface TaskOverviewSource {
  subscribe: (listener: TaskOverviewListener) => () => void;
  refreshNow: () => Promise<void>;
  getLatest: () => ApplicationTask[] | null;
  getLastError: () => unknown;
}

/**
 * 计算完整任务总览快照签名（复用 computeTasksSignature 基础并扩展进度与时间戳字段）
 */
export function computeTaskOverviewSignature(
  tasks: ReadonlyArray<ApplicationTask>
): string {
  if (!Array.isArray(tasks) || tasks.length === 0) return "";
  const baseSig = computeTasksSignature(tasks as ApplicationTask[]);
  const detailSig = tasks
    .map(
      (t) =>
        `${t.task_id}:${t.done_bytes}:${t.total_bytes ?? ""}:${t.updated_at ?? ""}:${t.bytes_per_second ?? ""}:${t.speed_formatted ?? ""}:${t.completed_items_count ?? ""}:${t.failed_items_count ?? ""}:${JSON.stringify(t.pause_reason ?? null)}:${JSON.stringify(t.failure ?? null)}:${typeof t.error === "string" ? t.error : t.error?.code ?? ""}`
    )
    .sort()
    .join("|");
  return `${baseSig}#${detailSig}`;
}

/**
 * 判断前后两次任务总览快照是否发生实质变化（用于抑制空闲每秒无变化重渲染）
 */
export function taskSnapshotChanged(
  prev: ReadonlyArray<ApplicationTask> | null | undefined,
  next: ReadonlyArray<ApplicationTask>
): boolean {
  if (!Array.isArray(prev)) return true;
  if (prev === next) return false;
  if (prev.length !== next.length) return true;
  if (prev.length === 0 && next.length === 0) return false;
  return computeTaskOverviewSignature(prev) !== computeTaskOverviewSignature(next);
}

/**
 * 单一任务总览订阅源 (W5-8 P0-1 / R4c-2)
 * - 只要存在至少 1 个订阅者，持续按轮询间隔拉取（窗口可见时默认 1000 ms，窗口隐藏时降频为默认 10000 ms；回到可见立即触发一次 refreshNow 并恢复 1000 ms），绝不空闲停
 * - refreshNow() 立即触发一次拉取，并对并发调用做单飞合并（最多排队 1 次尾随拉取，绝不形成循环）；拉取失败时记录 lastError 供调用方感知
 * - 拉取异常保留上一份任务列表，绝不向轮询环外抛出未捕获错误
 * - 最后一个订阅者退订后立即清理全部定时器与可见性监听，容忍 React StrictMode 双挂载/卸载
 */
export function createTaskOverviewSource(
  options: TaskOverviewSourceOptions
): TaskOverviewSource {
  const {
    fetchTasks,
    pollIntervalMs = 1000,
    hiddenPollIntervalMs = 10000,
    timers = defaultTimerDependencies,
    visibility = defaultVisibilityDependencies,
  } = options;

  const listeners = new Set<TaskOverviewListener>();
  let latest: ApplicationTask[] | null = null;
  let lastError: unknown = null;
  let timerId: unknown = null;
  let activeIntervalMs: number | null = null;
  let unsubscribeVisibility: (() => void) | null = null;
  let inFlightPromise: Promise<void> | null = null;
  let trailingRequested = false;

  const getDesiredIntervalMs = (): number =>
    visibility.isHidden() ? hiddenPollIntervalMs : pollIntervalMs;

  const handleVisibilityChange = () => {
    if (listeners.size === 0) return;
    syncTimerAndVisibility();
    if (!visibility.isHidden()) {
      void refreshNow();
    }
  };

  const syncTimerAndVisibility = () => {
    if (listeners.size === 0) {
      if (timerId !== null) {
        timers.clearInterval(timerId);
        timerId = null;
        activeIntervalMs = null;
      }
      if (unsubscribeVisibility !== null) {
        unsubscribeVisibility();
        unsubscribeVisibility = null;
      }
      return;
    }

    if (unsubscribeVisibility === null) {
      unsubscribeVisibility = visibility.onVisibilityChange(handleVisibilityChange);
    }

    const desiredMs = getDesiredIntervalMs();
    if (timerId === null || activeIntervalMs !== desiredMs) {
      if (timerId !== null) {
        timers.clearInterval(timerId);
      }
      activeIntervalMs = desiredMs;
      timerId = timers.setInterval(() => {
        void refreshNow();
      }, desiredMs);
    }
  };

  const notifyListeners = (list: ApplicationTask[]) => {
    for (const listener of Array.from(listeners)) {
      try {
        listener(list);
      } catch {
        // 隔离单个订阅者回调异常，保证其余订阅者与轮询环正常运转
      }
    }
  };

  const refreshNow = (): Promise<void> => {
    if (inFlightPromise !== null) {
      trailingRequested = true;
      return inFlightPromise;
    }

    const runCycle = async (): Promise<void> => {
      try {
        do {
          trailingRequested = false;
          try {
            const fresh = await fetchTasks();
            latest = fresh;
            lastError = null;
            notifyListeners(fresh);
          } catch (err) {
            // 拉取错误保留上一份列表，不向轮询环外抛出，但记录 lastError 供显式刷新调用方感知失败
            lastError = err ?? new Error(t("shared.taskOverviewSource.refreshFailedError"));
          }
        } while (trailingRequested);
      } finally {
        inFlightPromise = null;
      }
    };

    inFlightPromise = runCycle();
    return inFlightPromise;
  };

  const subscribe = (listener: TaskOverviewListener): (() => void) => {
    let active = true;
    listeners.add(listener);
    syncTimerAndVisibility();
    void refreshNow();

    return () => {
      if (!active) return;
      active = false;
      listeners.delete(listener);
      syncTimerAndVisibility();
    };
  };

  return {
    subscribe,
    refreshNow,
    getLatest: () => latest,
    getLastError: () => lastError,
  };
}
