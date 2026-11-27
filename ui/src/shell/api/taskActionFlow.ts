import { t } from "../../i18n/index.ts";
import type { ApplicationTask, RowAction } from "./types.ts";
import { explainActionError } from "./errorExplanations.ts";

export type TaskActionKind = RowAction | "pause" | "resume" | "retry" | "cancel" | "archive";

export interface TaskActionRunnerDependencies {
  taskAction: (taskId: string, revision: number, action: TaskActionKind) => Promise<unknown>;
  fetchTasks: () => Promise<ApplicationTask[]>;
}

export interface TaskActionRunner {
  run: (taskId: string, action: TaskActionKind, knownRevision?: number | null) => Promise<void>;
}

const STORE_REVISION_CONFLICT_CODE = "V3_STORE_REVISION_CONFLICT";

export function isKnownRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0;
}

function extractConflictOrErrorCode(err: unknown): string | undefined {
  if (typeof err === "string") {
    if (err.includes(STORE_REVISION_CONFLICT_CODE)) {
      return STORE_REVISION_CONFLICT_CODE;
    }
    return undefined;
  }
  if (err && typeof err === "object") {
    const rec = err as Record<string, unknown>;
    if (typeof rec.code === "string" && rec.code.length > 0) {
      return rec.code;
    }
    if (typeof rec.error_code === "string" && rec.error_code.length > 0) {
      return rec.error_code;
    }
    if (rec.error && typeof rec.error === "object") {
      const nested = rec.error as Record<string, unknown>;
      if (typeof nested.code === "string" && nested.code.length > 0) {
        return nested.code;
      }
    }
    if (typeof rec.message === "string" && rec.message.includes(STORE_REVISION_CONFLICT_CODE)) {
      return STORE_REVISION_CONFLICT_CODE;
    }
  }
  return undefined;
}

function isRevisionConflictError(err: unknown): boolean {
  return extractConflictOrErrorCode(err) === STORE_REVISION_CONFLICT_CODE;
}

function createTaskNotFoundError(): Error & { code: "TASK_NOT_FOUND"; message_zh: string } {
  const messageZh = t("shared.taskActionFlow.taskMissingOrEnded");
  return Object.assign(new Error(messageZh), {
    code: "TASK_NOT_FOUND" as const,
    message_zh: messageZh,
  });
}

export function resolveTaskActionErrorText(
  err: unknown,
  fallbackZh = t("shared.taskActionFlow.actionFailedFallback")
): string {
  if (
    err &&
    typeof err === "object" &&
    typeof (err as { message_zh?: unknown }).message_zh === "string"
  ) {
    const trimmed = (err as { message_zh: string }).message_zh.trim();
    if (trimmed.length > 0) {
      return trimmed;
    }
  }
  return explainActionError(err, fallbackZh);
}

function toChineseActionError(
  err: unknown
): Error & { code?: string; message_zh: string; cause?: unknown } {
  const code = extractConflictOrErrorCode(err);
  const fallbackZh =
    code === STORE_REVISION_CONFLICT_CODE ? t("shared.taskActionFlow.statusUpdatedRetry") : t("shared.taskActionFlow.actionFailedFallback");
  const explainedZh =
    err &&
    typeof err === "object" &&
    typeof (err as { message_zh?: unknown }).message_zh === "string" &&
    (err as { message_zh: string }).message_zh.trim().length > 0
      ? (err as { message_zh: string }).message_zh.trim()
      : explainActionError(code ? { code } : err, fallbackZh);
  return Object.assign(new Error(explainedZh, { cause: err }), {
    ...(code ? { code } : {}),
    message_zh: explainedZh,
    cause: err,
  });
}

/**
 * 创建任务动作执行器 (W5-8 P0-2 / R4c-1)
 * - 已知非负整数修订号（>= 0）时优先使用；缺失或无效时先经 fetchTasks() 查询真实修订号，绝不编造修订号
 * - 若查询后任务仍不存在，直接抛出 TASK_NOT_FOUND，不调用底层 taskAction
 * - 若底层返回修订冲突，自动重新拉取最新修订号并仅重试一次；再次失败则抛出简体中文错误
 */
export function createTaskActionRunner(
  deps: TaskActionRunnerDependencies
): TaskActionRunner {
  const { taskAction, fetchTasks } = deps;

  const lookupFreshRevision = async (taskId: string): Promise<number> => {
    const freshTasks = await fetchTasks();
    const matched = freshTasks.find((t) => t.task_id === taskId);
    if (!matched || !isKnownRevision(matched.revision)) {
      throw createTaskNotFoundError();
    }
    return matched.revision;
  };

  return {
    run: async (taskId: string, action: TaskActionKind, knownRevision?: number | null) => {
      const initialRevision = isKnownRevision(knownRevision)
        ? knownRevision
        : await lookupFreshRevision(taskId);

      try {
        await taskAction(taskId, initialRevision, action);
        return;
      } catch (firstErr: unknown) {
        if (!isRevisionConflictError(firstErr)) {
          throw toChineseActionError(firstErr);
        }
      }

      const freshRevision = await lookupFreshRevision(taskId);
      try {
        await taskAction(taskId, freshRevision, action);
      } catch (retryErr: unknown) {
        throw toChineseActionError(retryErr);
      }
    },
  };
}

/** 「打开目录」失败的中文提示：后端中文原因 → 修订号不符（任务刚变化）→ 已知错误码 →「未能打开保存目录」。 */
export function openTaskOutputErrorText(err: unknown, buttonLabelZh = t("shared.taskActionFlow.defaultOpenDirLabel")): string {
  if (
    err &&
    typeof err === "object" &&
    typeof (err as { message_zh?: unknown }).message_zh === "string" &&
    (err as { message_zh: string }).message_zh.trim().length > 0
  ) {
    return (err as { message_zh: string }).message_zh.trim();
  }
  if (isRevisionConflictError(err)) return t("shared.taskActionFlow.statusChangedClickAgain", { buttonLabel: buttonLabelZh });
  return resolveTaskActionErrorText(err, t("shared.taskActionFlow.openSaveDirFailed"));
}
