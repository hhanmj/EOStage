import type {
  TaskConflictFile,
  TaskConflictFileDeleteOutcome,
  TaskConflictFileExpectation,
} from "./generated/v3-contracts.ts";
import type { ApplicationTask } from "../shell/api/types.ts";

export function taskConflictCleanupIsActive(
  task: Pick<ApplicationTask, "status" | "scene_items">,
): boolean {
  return task.status === "queued" || task.status === "running"
    || Boolean(task.scene_items?.some((item) => item.status === "queued" || item.status === "running"));
}

export function conflictFileExpectations(
  files: readonly TaskConflictFile[],
): TaskConflictFileExpectation[] {
  return files.map(({ path, size_bytes, modified_at_epoch_ms }) => ({
    path,
    size_bytes,
    modified_at_epoch_ms,
  }));
}

export function conflictFilePreview(files: readonly TaskConflictFile[], limit = 5) {
  return {
    files: files.slice(0, limit),
    hiddenCount: Math.max(0, files.length - limit),
  };
}

export function conflictDeleteSummary(outcomes: readonly TaskConflictFileDeleteOutcome[]) {
  const deleted = outcomes.filter((outcome) => outcome.outcome === "deleted").length;
  return { deleted, skipped: outcomes.length - deleted };
}

export function conflictCleanupErrorText(error: unknown, fallback: string): string {
  if (error && typeof error === "object") {
    const value = error as Record<string, unknown>;
    const detail = value.error ?? value.message ?? value.code;
    if (typeof detail === "string" && detail.trim()) return `${fallback}: ${detail.trim()}`;
  } else if (typeof error === "string" && error.trim()) {
    return `${fallback}: ${error.trim()}`;
  }
  return fallback;
}
