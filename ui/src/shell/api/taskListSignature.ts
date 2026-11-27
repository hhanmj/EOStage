import type { ApplicationTask } from "./types.ts";

/**
 * 任务列表签名：只由 (task_id, status, revision) 决定，与列表顺序无关。
 * 任务轮询用它判断任务集合是否变化，变化时才重读 Agent 活动状态 (C1 规则 12)。
 */
export function taskListSignature(
  tasks: ReadonlyArray<Pick<ApplicationTask, "task_id" | "status" | "revision">> | null | undefined
): string {
  if (!Array.isArray(tasks)) return "";
  const parts: string[] = [];
  for (const t of tasks) {
    if (!t || typeof t.task_id !== "string") continue;
    parts.push(JSON.stringify([t.task_id, t.status, t.revision]));
  }
  parts.sort();
  return parts.join("\n");
}
