/**
 * 已批准确认卡的下载目录（进程内存，按 plan_id）。
 * 确认卡快照里的 output_dir 是批准前的值，卡上选的目录只随批准请求发给后端；
 * 对话页离开再回来时卡片重新挂载，靠这里显示批准时实际使用的目录。不跨重启。
 */
import type { ApplicationTask } from "./types.ts";
import { taskOutputDir } from "./taskLogic.ts";

const approvedDirs = new Map<string, string>();

export function rememberApprovedOutputDir(planId: string, dir: string | null | undefined): void {
  if (typeof planId !== "string" || !planId) return;
  if (typeof dir !== "string" || dir.trim().length === 0) return;
  approvedDirs.set(planId, dir);
}

export function recallApprovedOutputDir(planId: string): string | null {
  return approvedDirs.get(planId) ?? null;
}

/**
 * 已批准卡显示的目录：批准时的记录优先；没有记录（如重启后）时用本卡已建任务的保存目录
 * （任务总览 output_root）；都没有或为空串时为 null，卡上写「未知」。
 */
export function resolveApprovedOutputDir(
  memoryDir: string | null | undefined,
  createdTaskIds: readonly string[],
  tasks: readonly ApplicationTask[],
): string | null {
  if (typeof memoryDir === "string" && memoryDir.trim().length > 0) return memoryDir;
  const ids = new Set(createdTaskIds);
  for (const task of tasks) {
    if (!ids.has(task.task_id)) continue;
    const dir = taskOutputDir(task);
    if (dir !== null) return dir;
  }
  return null;
}

export function clearApprovedOutputDirs(): void {
  approvedDirs.clear();
}
