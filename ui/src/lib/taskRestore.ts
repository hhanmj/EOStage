import type { DownloadBatchContext } from "./bridge.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";
import {
  isSentinel2L2aCatalogProduct,
  SENTINEL2_L2A_LOGICAL_DATASET,
  SENTINEL2_L2A_PROVIDER_ID,
} from "./sentinel2Catalog.ts";
import {
  isLandsatC2L2CatalogProduct,
  LANDSAT_C2_L2_LOGICAL_DATASET,
  isLandsatProviderId,
} from "./landsatCatalog.ts";

const RESTORABLE_HISTORY_STATES = new Set([
  "failed",
  "interrupted",
  "timeout",
  "cancelled",
]);

export function isRestorableHistoryStatus(status: string): boolean {
  return RESTORABLE_HISTORY_STATES.has(status);
}

export function restoredBatchContext(
  context: DownloadBatchContext | null | undefined,
  parentTaskId: string,
): DownloadBatchContext | null {
  if (!context) return null;
  const cleanParentTaskId = parentTaskId.trim();
  if (!cleanParentTaskId) return context;
  return {
    ...context,
    query: {
      ...context.query,
      restored_from_task_id: cleanParentTaskId,
    },
  };
}

export type OpticalHistoryRestoreRoute = "sentinel2" | "landsat" | "unsupported_optical" | null;

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function cleanHistoryId(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

/** A provider/source hint keeps malformed optical history out of generic SAR restore. */
export function isOpticalHistoryProductHint(product: SarProduct | null | undefined): boolean {
  if (!product) return false;
  const metadata = record(product.native_metadata);
  const provider = cleanHistoryId(product.provider);
  return provider === SENTINEL2_L2A_PROVIDER_ID
    || isLandsatProviderId(provider)
    || metadata?.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET
    || metadata?.logical_dataset === LANDSAT_C2_L2_LOGICAL_DATASET
    || metadata?.source_kind === "sentinel2"
    || metadata?.source_kind === "landsat"
    || metadata?.product_kind === "optical_scene";
}

/** Classify only immutable, provider-bound optical task products for history restore. */
export function opticalHistoryRestoreRoute(
  products: readonly SarProduct[],
): OpticalHistoryRestoreRoute {
  if (products.length === 0) return null;
  if (products.every(isSentinel2L2aCatalogProduct)) return "sentinel2";
  if (products.every(isLandsatC2L2CatalogProduct)) return "landsat";
  const hasOpticalMetadata = products.some(isOpticalHistoryProductHint);
  return hasOpticalMetadata ? "unsupported_optical" : null;
}

export type OpticalHistoryRestoreSelectionRow = {
  row_id: string;
  item_id: string;
  product: SarProduct | null;
  /** Used when a legacy row has no complete product object but still identifies optical data. */
  optical_hint?: boolean;
};

export type OpticalHistoryRestoreSelectionResult = {
  valid: boolean;
  rows: OpticalHistoryRestoreSelectionRow[];
  item_ids: string[];
  products: SarProduct[];
  route: OpticalHistoryRestoreRoute;
  reason:
    | "missing_selection"
    | "duplicate_selection"
    | "missing_row"
    | "ambiguous_row"
    | "missing_item_id"
    | "duplicate_mapping"
    | "missing_product"
    | "unsupported_optical"
    | null;
};

/** Resolve the selected task membership before classifying its provider route. */
export function resolveOpticalHistoryRestoreSelection(
  selectedIds: readonly string[],
  rows: readonly OpticalHistoryRestoreSelectionRow[],
): OpticalHistoryRestoreSelectionResult {
  const normalizedIds = selectedIds.map(cleanHistoryId);
  if (normalizedIds.length === 0 || normalizedIds.some((value) => !value)) {
    return {
      valid: false,
      rows: [],
      item_ids: [],
      products: [],
      route: null,
      reason: "missing_selection",
    };
  }
  if (new Set(normalizedIds).size !== normalizedIds.length) {
    return {
      valid: false,
      rows: [],
      item_ids: [],
      products: [],
      route: null,
      reason: "duplicate_selection",
    };
  }

  const selectedRows: OpticalHistoryRestoreSelectionRow[] = [];
  const seenRowIds = new Set<string>();
  const seenItemIds = new Set<string>();
  for (const selectedId of normalizedIds) {
    const matches = rows.filter((row) => {
      const rowId = cleanHistoryId(row.row_id);
      const itemId = cleanHistoryId(row.item_id);
      return rowId === selectedId || itemId === selectedId;
    });
    if (matches.length === 0) {
      return {
        valid: false,
        rows: [],
        item_ids: [],
        products: [],
        route: null,
        reason: "missing_row",
      };
    }
    if (matches.length !== 1) {
      return {
        valid: false,
        rows: [],
        item_ids: [],
        products: [],
        route: null,
        reason: "ambiguous_row",
      };
    }
    const row = matches[0];
    const rowId = cleanHistoryId(row.row_id);
    const itemId = cleanHistoryId(row.item_id);
    if (!rowId || !itemId) {
      return {
        valid: false,
        rows: [],
        item_ids: [],
        products: [],
        route: null,
        reason: "missing_item_id",
      };
    }
    if (seenRowIds.has(rowId) || seenItemIds.has(itemId)) {
      return {
        valid: false,
        rows: [],
        item_ids: [],
        products: [],
        route: null,
        reason: "duplicate_mapping",
      };
    }
    seenRowIds.add(rowId);
    seenItemIds.add(itemId);
    selectedRows.push({ ...row, row_id: rowId, item_id: itemId });
  }

  const products = selectedRows
    .map((row) => row.product)
    .filter((product): product is SarProduct => Boolean(product));
  const missingProduct = products.length !== selectedRows.length;
  const opticalHint = selectedRows.some((row) => (
    row.optical_hint === true || isOpticalHistoryProductHint(row.product)
  ));
  const route = missingProduct
    ? opticalHint ? "unsupported_optical" : null
    : opticalHistoryRestoreRoute(products);
  return {
    valid: true,
    rows: selectedRows,
    item_ids: selectedRows.map((row) => row.item_id),
    products,
    route,
    reason: missingProduct
      ? "missing_product"
      : route === "unsupported_optical"
        ? "unsupported_optical"
        : null,
  };
}
