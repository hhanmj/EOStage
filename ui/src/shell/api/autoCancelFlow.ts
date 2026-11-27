import { t } from "../../i18n/index.ts";
import { planAutoCancel } from "./approvalLogic.ts";
import type { ApplicationTask } from "./types.ts";

export interface AutoCancelDeps {
  getFreshTasks: () => Promise<ApplicationTask[]>;
  /** 返回 false 表示后端该运行已结束（没有可取消的运行）；抛错才是取消失败 */
  runCancel: (runId: string) => Promise<boolean | void>;
  cancelTask: (taskId: string, revision: number) => Promise<void>;
  runLive: boolean;
  /** 可选：调用方传入的初始（可能陈旧）任务快照，仅用于对照，决策始终以 getFreshTasks() 为准 */
  initialTasks?: ApplicationTask[];
  runId?: string;
  createdTaskIds?: string[];
}

export interface AutoCancelInput {
  runId: string;
  createdTaskIds: string[];
}

export interface AutoCancelExecutionResult {
  runCancelled: boolean;
  cancelledTaskIds: string[];
  failedTaskIds: string[];
  alreadyEndedTaskIds: string[];
  skipped: { taskId: string; reason: "not_found" | "not_active" }[];
  refreshFailed: boolean;
  outcomeText: string;
}

/**
 * W5-8 P0-3 / R4c-1 自动执行卡取消编排器
 * 1. 总是先调用 getFreshTasks() 获取最新任务快照，再以 planAutoCancel 计算取消目标；
 * 2. 若 getFreshTasks() 抛出异常，仍执行当前运行取消，但任务部分如实报告「未能确认任务状态」，不得声称任务已取消；
 * 3. 若刷新成功但部分 createdTaskIds 处于 not_found 或 not_active，记入 alreadyEndedTaskIds（此前已结束），不调用全局停止，也不显示原始任务 ID；
 * 4. 对 plan.cancelTasks 逐个调用 cancelTask(taskId, revision)，单任务失败记录入 failedTaskIds。
 * 5. runCancel 返回 false 表示运行此前已结束：不算取消成功也不算失败，文案「本次运行已结束」。
 */
export async function executeAutoCancel(
  deps: AutoCancelDeps,
  input?: AutoCancelInput
): Promise<AutoCancelExecutionResult> {
  const runId = input?.runId ?? deps.runId ?? "";
  const createdTaskIds = input?.createdTaskIds ?? deps.createdTaskIds ?? [];

  let freshTasks: ApplicationTask[] = [];
  let refreshFailed = false;
  try {
    const fetched = await deps.getFreshTasks();
    if (Array.isArray(fetched)) {
      freshTasks = fetched;
    } else {
      refreshFailed = true;
    }
  } catch {
    refreshFailed = true;
    freshTasks = [];
  }

  let runCancelled = false;
  let runAlreadyEnded = false;
  if (deps.runLive) {
    try {
      const cancelled = await deps.runCancel(runId);
      if (cancelled === false) {
        runAlreadyEnded = true;
      } else {
        runCancelled = true;
      }
    } catch {
      // runCancel 失败不中止后续处理
    }
  }

  // 运行仍在进行但取消请求失败：如实说明，不得落到「都已结束，无需取消」
  const runCancelFailed = deps.runLive && !runCancelled && !runAlreadyEnded;

  if (refreshFailed) {
    const outcomeText = runCancelled
      ? t("shared.autoCancelFlow.cancelledRunTaskUnconfirmed")
      : runCancelFailed
        ? t("shared.autoCancelFlow.cancelRunFailedTaskUnconfirmed")
        : runAlreadyEnded
          ? t("shared.autoCancelFlow.runEndedTaskUnconfirmed")
          : t("shared.autoCancelFlow.taskUnconfirmed");
    return {
      runCancelled,
      cancelledTaskIds: [],
      failedTaskIds: [],
      alreadyEndedTaskIds: [],
      skipped: [],
      refreshFailed: true,
      outcomeText,
    };
  }

  const plan = planAutoCancel({
    runId,
    runLive: deps.runLive,
    createdTaskIds,
    tasks: freshTasks,
  });

  const cancelledTaskIds: string[] = [];
  const failedTaskIds: string[] = [];

  for (const item of plan.cancelTasks) {
    try {
      await deps.cancelTask(item.taskId, item.revision);
      if (!cancelledTaskIds.includes(item.taskId)) {
        cancelledTaskIds.push(item.taskId);
      }
    } catch {
      if (!failedTaskIds.includes(item.taskId)) {
        failedTaskIds.push(item.taskId);
      }
    }
  }

  const alreadyEndedTaskIds: string[] = [];
  for (const sk of plan.skipped) {
    if (!alreadyEndedTaskIds.includes(sk.taskId)) {
      alreadyEndedTaskIds.push(sk.taskId);
    }
  }

  const parts: string[] = [];
  if (runCancelled && cancelledTaskIds.length > 0) {
    parts.push(t("shared.autoCancelFlow.cancelledRunAndTasks", { count: cancelledTaskIds.length }));
  } else if (!runCancelled && cancelledTaskIds.length > 0) {
    if (runAlreadyEnded) parts.push(t("shared.autoCancelFlow.runAlreadyEnded"));
    parts.push(t("shared.autoCancelFlow.cancelledTasksOnly", { count: cancelledTaskIds.length }));
  } else if (runCancelled && cancelledTaskIds.length === 0) {
    parts.push(t("shared.autoCancelFlow.cancelledRunOnly"));
  } else if (
    runAlreadyEnded &&
    !(alreadyEndedTaskIds.length > 0 && failedTaskIds.length === 0)
  ) {
    // 任务都已结束且无失败时由下面的「本次运行与任务都已结束，无需取消」覆盖
    parts.push(t("shared.autoCancelFlow.runAlreadyEnded"));
  }

  if (alreadyEndedTaskIds.length > 0) {
    if (parts.length > 0 || runCancelFailed) {
      parts.push(t("shared.autoCancelFlow.tasksPreviouslyEnded", { count: alreadyEndedTaskIds.length }));
    } else if (failedTaskIds.length === 0) {
      parts.push(t("shared.autoCancelFlow.allAlreadyEnded"));
    }
  }

  if (runCancelFailed) {
    parts.push(t("shared.autoCancelFlow.failedToCancelRun"));
  }

  if (failedTaskIds.length > 0) {
    parts.push(t("shared.autoCancelFlow.failedToCancelTasks", { count: failedTaskIds.length }));
  }

  const outcomeText =
    parts.length > 0 ? parts.join(t("shared.autoCancelFlow.partSeparator")) : t("shared.autoCancelFlow.allAlreadyEnded");

  return {
    runCancelled,
    cancelledTaskIds,
    failedTaskIds,
    alreadyEndedTaskIds,
    skipped: [...plan.skipped],
    refreshFailed: false,
    outcomeText,
  };
}
