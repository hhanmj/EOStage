/**
 * sessionDownloadScope.ts
 * Computes the count of genuinely active running background download task groups
 * owned by the CURRENT Agent session — never counting unrelated, historical,
 * paused, or other-session tasks when the current conversation created zero downloads.
 */

import type { AgentStreamEvent, AgentWorkItem } from "@/shell/api/agentBridgeTypes";
import type { ApplicationTask } from "@/shell/api/types";
import { agentCreatedTaskIds } from "@/shell/api/stopAllLogic";
import { effectiveGroupStatus, groupOrbitCompanionTasks } from "@/shell/api/orbitChildRows";

export function collectSessionOwnedTaskIds(
  events: AgentStreamEvent[],
  sessionWork?: readonly AgentWorkItem[] | null
): Set<string> {
  const ids = new Set<string>(agentCreatedTaskIds(events));
  if (sessionWork) {
    for (const item of sessionWork) {
      if (!item || !Array.isArray(item.created_task_ids)) continue;
      for (const tid of item.created_task_ids) {
        if (typeof tid === "string" && tid.trim().length > 0) {
          ids.add(tid.trim());
        }
      }
    }
  }
  return ids;
}

export function countSessionActiveDownloads(params: {
  activeSessionId: string | null | undefined;
  events: AgentStreamEvent[];
  sessionWork?: readonly AgentWorkItem[] | null;
  tasks: readonly ApplicationTask[];
}): number {
  const { activeSessionId, events, sessionWork, tasks } = params;
  if (!activeSessionId || !Array.isArray(tasks) || tasks.length === 0) {
    return 0;
  }

  const ownedIds = collectSessionOwnedTaskIds(events, sessionWork);
  if (ownedIds.size === 0) {
    return 0;
  }

  const ownedTasks = tasks.filter(
    (task) =>
      ownedIds.has(task.task_id) ||
      (typeof task.source_task_id === "string" && ownedIds.has(task.source_task_id))
  );
  if (ownedTasks.length === 0) {
    return 0;
  }

  return groupOrbitCompanionTasks(ownedTasks).filter((group) => {
    const status = effectiveGroupStatus(group);
    return status === "running";
  }).length;
}
