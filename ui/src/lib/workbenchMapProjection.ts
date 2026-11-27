export const GOLDEN_MAP_SCENE_LIMIT = 200;

export type WorkbenchMapProjectionContext = {
  batch_id: string;
  revision: number | null | undefined;
  candidate_count: number | null | undefined;
  dock_page_size: number;
};

export type WorkbenchMapProjectionRequest = {
  batch_id: string;
  expected_revision: number;
  page_index: 0;
  page_size: number;
};

/**
 * The dock's virtual page and the map's bounded overview are separate read
 * projections of one Store-owned SearchBatch. This never mutates Workbench
 * selection or page state.
 */
export function workbenchMapProjectionRequest(
  context: WorkbenchMapProjectionContext,
): WorkbenchMapProjectionRequest | null {
  const batchId = context.batch_id.trim();
  const revision = context.revision;
  const candidateCount = context.candidate_count;
  const dockPageSize = context.dock_page_size;
  if (
    !batchId
    || !Number.isSafeInteger(revision)
    || (revision as number) < 0
    || !Number.isSafeInteger(candidateCount)
    || (candidateCount as number) <= 0
    || !Number.isSafeInteger(dockPageSize)
    || dockPageSize <= 0
    || dockPageSize > GOLDEN_MAP_SCENE_LIMIT
  ) return null;
  return {
    batch_id: batchId,
    expected_revision: revision as number,
    page_index: 0,
    page_size: Math.min(candidateCount as number, GOLDEN_MAP_SCENE_LIMIT),
  };
}
