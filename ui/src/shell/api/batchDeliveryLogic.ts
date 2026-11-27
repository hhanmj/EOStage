/**
 * #22 B slice 5: how the map workbench presents a progressively delivered
 * search batch (contract 3.41.0). Pure functions so counts, paging and the
 * select-all gate are tested without rendering.
 *
 * A batch is `loading` while the provider keeps streaming rows into it; it
 * ends `complete`, or `cancelled` / `failed` / `interrupted` with the rows
 * that arrived kept (possibly incomplete).
 */
import type { SearchBatchProgressState, SearchProgress } from "../../lib/generated/v3-contracts.ts";

/** The 10,000-scene safety ceiling a single search may keep. */
export const SEARCH_RESULT_CEILING = 10_000;

/** Rows shown per result page in the map workbench (up to backend MAX_WORKBENCH_PAGE_SIZE = 250). */
export const RESULT_PAGE_SIZE = 250;

export interface BatchDelivery {
  batchId: string;
  state: SearchBatchProgressState;
  /** Rows that have arrived (for a sealed batch: its frozen count). */
  loadedCount: number;
  /** Provider total when the provider reports one. */
  remoteTotal: number | null;
}

export interface DeliveryCounts {
  /** The headline 「N 景」: the provider total while loading, else the frozen count. */
  headline: number;
  /** While loading: rows arrived so far and the total they count toward. */
  loaded: { loaded: number; total: number | null } | null;
  /** The provider reports more matches than the ceiling keeps. */
  overCeiling: { total: number; limit: number } | null;
  /** How many rows paging may eventually reach. */
  pageable: number;
}

export function isLoading(delivery: BatchDelivery | null | undefined): boolean {
  return delivery?.state === "loading";
}

/** A sealed batch that stopped before the provider finished keeps its rows, possibly incomplete. */
export function isStoppedEarly(delivery: BatchDelivery | null | undefined): boolean {
  return delivery?.state === "cancelled"
    || delivery?.state === "failed"
    || delivery?.state === "interrupted";
}

export function deliveryCounts(
  delivery: BatchDelivery | null | undefined,
  frozenTotal: number,
): DeliveryCounts {
  const remoteTotal = delivery?.remoteTotal ?? null;
  const overCeiling = remoteTotal !== null && remoteTotal > SEARCH_RESULT_CEILING
    ? { total: remoteTotal, limit: SEARCH_RESULT_CEILING }
    : null;
  if (!delivery || delivery.state !== "loading") {
    return { headline: frozenTotal, loaded: null, overCeiling, pageable: frozenTotal };
  }
  const loaded = Math.max(delivery.loadedCount, 0);
  const total = remoteTotal === null ? null : Math.max(remoteTotal, loaded);
  return {
    headline: total ?? loaded,
    loaded: { loaded, total },
    overCeiling,
    pageable: Math.max(loaded, Math.min(total ?? loaded, SEARCH_RESULT_CEILING)),
  };
}

/** Select-all over every matching scene waits until the batch has finished loading. */
export function selectAllMatchingAvailable(delivery: BatchDelivery | null | undefined): boolean {
  return !isLoading(delivery);
}

/**
 * The batch a non-terminal search is filling, once its first page is
 * readable; `null` before page 1 has arrived or for another source.
 */
export function loadingBatchOf(
  progress: SearchProgress | null | undefined,
  resourceId: string,
): { requestId: string; batchId: string } | null {
  if (!progress || progress.terminal || progress.resource_id !== resourceId) return null;
  const batchId = progress.batch_id;
  if (typeof batchId !== "string" || batchId.length === 0) return null;
  return { requestId: progress.request_id, batchId };
}

/** Folds a live progress update into the delivery of the batch it fills. */
export function deliveryFromProgress(
  previous: BatchDelivery | null,
  progress: SearchProgress,
): BatchDelivery | null {
  const batchId = progress.batch_id;
  if (typeof batchId !== "string" || batchId.length === 0) return previous;
  const base = previous?.batchId === batchId ? previous : null;
  let state: SearchBatchProgressState = base?.state ?? "loading";
  if (!progress.terminal) {
    state = "loading";
  } else if (progress.stage === "done") {
    state = "complete";
  } else if (progress.stage === "cancelled") {
    state = "cancelled";
  } else if (progress.stage === "failed") {
    state = "failed";
  }
  return {
    batchId,
    state,
    loadedCount: progress.loaded_count ?? base?.loadedCount ?? 0,
    remoteTotal: progress.remote_total ?? base?.remoteTotal ?? null,
  };
}

/**
 * The state of one result page: `ready` when its rows are here, `partial`
 * when some rows are here and more are expected, `pending` when none are.
 */
export function pageArrival(
  shownCount: number,
  page: number,
  pageable: number,
  pageSize = RESULT_PAGE_SIZE,
): "ready" | "partial" | "pending" {
  const start = (Math.max(1, page) - 1) * pageSize;
  const expectedEnd = Math.min(start + pageSize, Math.max(pageable, shownCount));
  if (shownCount >= expectedEnd) return "ready";
  return shownCount > start ? "partial" : "pending";
}

/** The delivery state a batch page reports (older pages without a state read as complete). */
export function deliveryFromPage(page: {
  batch_id: string;
  total_count: number;
  state?: SearchBatchProgressState;
  loaded_count?: number;
  remote_total?: number | null;
}): BatchDelivery {
  return {
    batchId: page.batch_id,
    state: page.state ?? "complete",
    loadedCount: page.loaded_count ?? page.total_count,
    remoteTotal: page.remote_total ?? null,
  };
}
