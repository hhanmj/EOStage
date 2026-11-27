export const MAX_CATALOG_PAGE_ITEMS = 250;
export const DEFAULT_WORKSPACE_PAGE_SIZE = MAX_CATALOG_PAGE_ITEMS;
// Task detail/history pages retain the bounded task-page contract; catalog
// presentation capacity must not change their request size.
export const DEFAULT_TASK_PAGE_SIZE = 100;
export const SCENE_WORKSPACE_ROW_HEIGHT = 68;

function nonNegativeCount(value: number) {
  return Number.isSafeInteger(value) && value > 0 ? value : 0;
}

export function catalogPaginationTotal({
  workingSetCount,
  filteredCount,
}: {
  workingSetCount: number;
  filteredCount: number | null;
}) {
  return filteredCount === null
    ? nonNegativeCount(workingSetCount)
    : nonNegativeCount(filteredCount);
}

export function catalogPageCount(totalCount: number) {
  return Math.max(1, Math.ceil(nonNegativeCount(totalCount) / MAX_CATALOG_PAGE_ITEMS));
}

export function catalogPageItems(totalCount: number, pageIndex: number) {
  const total = nonNegativeCount(totalCount);
  const page = Math.max(0, Math.trunc(pageIndex) || 0);
  return Math.max(0, Math.min(MAX_CATALOG_PAGE_ITEMS, total - page * MAX_CATALOG_PAGE_ITEMS));
}

function nonNegativeFinite(value: number) {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

export function pageIndexFromScrollTop(
  scrollTop: number,
  pageSize = DEFAULT_WORKSPACE_PAGE_SIZE,
  rowHeight = SCENE_WORKSPACE_ROW_HEIGHT,
) {
  const safePageSize = Math.max(1, Math.floor(nonNegativeFinite(pageSize)) || 1);
  const safeRowHeight = Math.max(1, nonNegativeFinite(rowHeight) || 1);
  return Math.floor(nonNegativeFinite(scrollTop) / (safePageSize * safeRowHeight));
}

export function scrollTopFromPageIndex(
  pageIndex: number,
  pageSize = DEFAULT_WORKSPACE_PAGE_SIZE,
  rowHeight = SCENE_WORKSPACE_ROW_HEIGHT,
) {
  const safePage = Math.floor(nonNegativeFinite(pageIndex));
  const safePageSize = Math.max(1, Math.floor(nonNegativeFinite(pageSize)) || 1);
  const safeRowHeight = Math.max(1, nonNegativeFinite(rowHeight) || 1);
  return safePage * safePageSize * safeRowHeight;
}
