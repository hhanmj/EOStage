import type { DownloadBatchContext } from "./bridge.ts";
import type { DesktopSarBackgroundCountRequest, DesktopSarBackgroundCountResult } from "./desktopShellBridge.ts";

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Merge a validated Store count fact, never an old page/selection snapshot. */
export function mergeFrozenSarMatchedCount(
  current: DownloadBatchContext | null,
  request: Pick<DesktopSarBackgroundCountRequest, "batch_id" | "expected_workbench_revision">,
  result: DesktopSarBackgroundCountResult,
): DownloadBatchContext | null {
  if (!current || !result.ok || !result.applied || !result.snapshot) return current;
  const total = result.total_count;
  const observed = record(result.snapshot.core.workbench);
  const stats = record(observed?.search_stats);
  if (
    total === null || !Number.isSafeInteger(total) || total < 0
    || current.batch_id !== request.batch_id
    || current.revision !== request.expected_workbench_revision
    || observed?.batch_id !== request.batch_id
    || observed?.revision !== request.expected_workbench_revision
    || stats?.matched_count !== total
    || total < (current.frozen_candidate_count ?? current.total_count ?? 0)
  ) return current;
  if (current.provider_matched_count === total) return current;
  // All immutable query/AOI data, paging and selection retain their current identity.
  return { ...current, provider_matched_count: total };
}
