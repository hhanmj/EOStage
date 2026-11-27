import type { AoiPreviewOk } from "./bridge.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";
import type { NisarSubsetSelection } from "./nisarSubset.ts";

export type NisarSubsetWorkspaceOverlapState = "unchecked" | "intersects" | "no_overlap" | "incompatible";

export class NisarSubsetPreflightGeneration {
  private generation = 0;

  begin(): number {
    this.generation += 1;
    return this.generation;
  }

  invalidate(): void {
    this.generation += 1;
  }

  isCurrent(generation: number): boolean {
    return generation === this.generation;
  }
}

export type NisarSubsetWorkspaceRow = {
  itemId: string;
  product: SarProduct;
  subsetEligible: boolean;
  blockedReason: string | null;
  overlapState: NisarSubsetWorkspaceOverlapState;
  exclusionReason?: "frequency_incompatible" | "polarization_incompatible";
  availableSelections: NisarSubsetSelection[];
};

export type NisarSubsetWorkspaceDraft = {
  sourceBatchId: string;
  selectedItemIds: string[];
  requestedGeometry: Record<string, unknown> | null;
  selectedFrequency: "A" | "B" | null;
  selectedPolarizations: string[];
};

export function createNisarSubsetWorkspaceDraft(
  sourceBatchId: string,
  rows: readonly NisarSubsetWorkspaceRow[],
  checkedIds: ReadonlySet<string>,
  highlightedId: string | null,
  workingGeometry: Record<string, unknown> | null,
): NisarSubsetWorkspaceDraft {
  const eligibleIds = new Set(
    rows.filter((row) => row.subsetEligible).map((row) => row.itemId),
  );
  const checked = [...checkedIds].filter((itemId) => eligibleIds.has(itemId));
  const selectedItemIds = checked.length > 0
    ? checked
    : highlightedId && eligibleIds.has(highlightedId)
      ? [highlightedId]
      : [];

  return {
    sourceBatchId,
    selectedItemIds,
    requestedGeometry: workingGeometry ? structuredClone(workingGeometry) : null,
    selectedFrequency: null,
    selectedPolarizations: [],
  };
}


export type NisarSubsetWorkspaceReadinessCode =
  | "ready"
  | "missing_items"
  | "missing_geometry"
  | "preflight_required"
  | "no_overlap"
  | "no_common_selection"
  | "capability_blocked";

export type NisarSubsetWorkspaceReadiness = {
  ready: boolean;
  code: NisarSubsetWorkspaceReadinessCode;
  reason: string | null;
  executableItemIds: string[];
  excludedItemIds: string[];
  conflictingItemIds: string[];
  overlapVerified: boolean;
};

function selectedRows(
  draft: NisarSubsetWorkspaceDraft,
  rows: readonly NisarSubsetWorkspaceRow[],
): NisarSubsetWorkspaceRow[] {
  const byId = new Map(rows.map((row) => [row.itemId, row]));
  return draft.selectedItemIds
    .map((itemId) => byId.get(itemId))
    .filter((row): row is NisarSubsetWorkspaceRow => row !== undefined);
}

function normalizedSelections(
  row: NisarSubsetWorkspaceRow,
): Map<"A" | "B", Set<string>> {
  const byFrequency = new Map<"A" | "B", Set<string>>();
  for (const selection of row.availableSelections) {
    const values = byFrequency.get(selection.frequency) ?? new Set<string>();
    for (const polarization of selection.polarizations) {
      const normalized = polarization.trim();
      if (normalized) values.add(normalized);
    }
    byFrequency.set(selection.frequency, values);
  }
  return byFrequency;
}

export function availableCommonNisarSelections(
  rows: readonly NisarSubsetWorkspaceRow[],
): NisarSubsetSelection[] {
  if (rows.length === 0 || rows.some((row) => !row.subsetEligible)) return [];
  const capabilities = rows.map(normalizedSelections);
  const common: NisarSubsetSelection[] = [];

  for (const frequency of ["A", "B"] as const) {
    const first = capabilities[0].get(frequency);
    if (!first) continue;
    const polarizations = [...first]
      .filter((polarization) => capabilities.every((values) => (
        values.get(frequency)?.has(polarization) === true
      )))
      .sort((left, right) => left.localeCompare(right));
    if (polarizations.length > 0) common.push({ frequency, polarizations });
  }

  return common;
}

function selectionIsAvailable(
  draft: NisarSubsetWorkspaceDraft,
  available: readonly NisarSubsetSelection[],
): boolean {
  if (!draft.selectedFrequency || draft.selectedPolarizations.length === 0) return false;
  const allowed = available.find((entry) => entry.frequency === draft.selectedFrequency);
  if (!allowed) return false;
  const allowedPolarizations = new Set(allowed.polarizations);
  const selected = new Set(draft.selectedPolarizations);
  return selected.size === draft.selectedPolarizations.length
    && [...selected].every((polarization) => allowedPolarizations.has(polarization));
}

function reconcileSelection(
  draft: NisarSubsetWorkspaceDraft,
  rows: readonly NisarSubsetWorkspaceRow[],
): NisarSubsetWorkspaceDraft {
  const available = availableCommonNisarSelections(selectedRows(draft, rows));
  return selectionIsAvailable(draft, available)
    ? draft
    : { ...draft, selectedFrequency: null, selectedPolarizations: [] };
}

export function toggleNisarSubsetWorkspaceItem(
  draft: NisarSubsetWorkspaceDraft,
  rows: readonly NisarSubsetWorkspaceRow[],
  itemId: string,
): NisarSubsetWorkspaceDraft {
  const existing = new Set(draft.selectedItemIds);
  if (existing.has(itemId)) {
    existing.delete(itemId);
  } else {
    const row = rows.find((candidate) => candidate.itemId === itemId);
    if (!row?.subsetEligible) return draft;
    existing.add(itemId);
  }
  return reconcileSelection({ ...draft, selectedItemIds: [...existing] }, rows);
}

export function setNisarSubsetWorkspaceGeometry(
  draft: NisarSubsetWorkspaceDraft,
  geometry: Record<string, unknown> | null,
): NisarSubsetWorkspaceDraft {
  return {
    ...draft,
    requestedGeometry: geometry ? structuredClone(geometry) : null,
  };
}

function readiness(
  code: NisarSubsetWorkspaceReadinessCode,
  reason: string | null,
  executableItemIds: string[],
  excludedItemIds: string[] = [],
  conflictingItemIds: string[] = [],
  overlapVerified = false,
): NisarSubsetWorkspaceReadiness {
  return {
    ready: code === "ready",
    code,
    reason,
    executableItemIds,
    excludedItemIds,
    conflictingItemIds,
    overlapVerified,
  };
}

export function nisarSubsetWorkspaceReadiness(
  draft: NisarSubsetWorkspaceDraft,
  rows: readonly NisarSubsetWorkspaceRow[],
): NisarSubsetWorkspaceReadiness {
  if (draft.selectedItemIds.length === 0) {
    return readiness("missing_items", "请至少选择一景影像", []);
  }
  const chosenRows = selectedRows(draft, rows);
  if (chosenRows.length !== draft.selectedItemIds.length) {
    const known = new Set(chosenRows.map((row) => row.itemId));
    const missing = draft.selectedItemIds.filter((itemId) => !known.has(itemId));
    return readiness("capability_blocked", "所选影像已不属于当前检索结果", [], [], missing);
  }
  const blocked = chosenRows.filter((row) => !row.subsetEligible).map((row) => row.itemId);
  if (blocked.length > 0) {
    return readiness("capability_blocked", "所选影像中包含不可裁剪项目", [], [], blocked);
  }
  if (!draft.requestedGeometry) {
    return readiness("missing_geometry", "请先选择裁剪范围", []);
  }
  if (chosenRows.some((row) => row.overlapState === "unchecked")) {
    return readiness("preflight_required", "请先检查相交与能力", []);
  }

  const excludedItemIds = chosenRows
    .filter((row) => row.overlapState === "no_overlap" || row.overlapState === "incompatible")
    .map((row) => row.itemId);
  const executableRows = chosenRows.filter((row) => row.overlapState === "intersects");
  const executableItemIds = executableRows.map((row) => row.itemId);
  if (executableRows.length === 0) {
    return readiness(
      chosenRows.some((row) => row.overlapState === "incompatible") ? "capability_blocked" : "no_overlap",
      chosenRows.some((row) => row.overlapState === "incompatible") ? "所选影像均不满足原任务的裁剪范围、频率或极化" : "当前 AOI 与所选影像均不相交",
      [],
      excludedItemIds,
      [],
      true,
    );
  }

  const available = availableCommonNisarSelections(executableRows);
  if (available.length === 0) {
    return readiness(
      "no_common_selection",
      "所选影像没有共同支持的频率和极化组合",
      executableItemIds,
      excludedItemIds,
      executableItemIds,
      executableRows.every((row) => row.overlapState === "intersects"),
    );
  }
  if (!selectionIsAvailable(draft, available)) {
    return readiness(
      "no_common_selection",
      "请选择所有可执行影像共同支持的频率和极化",
      executableItemIds,
      excludedItemIds,
      [],
      executableRows.every((row) => row.overlapState === "intersects"),
    );
  }

  return readiness(
    "ready",
    null,
    executableItemIds,
    excludedItemIds,
    [],
    executableRows.every((row) => row.overlapState === "intersects"),
  );
}


export type NisarSubsetBoundaryGeometryResult =
  | { ok: true; geometry: Record<string, unknown> }
  | { ok: false; error: string };

function recordValue(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function previewFeatureGeometry(
  preview: AoiPreviewOk,
  featureId: string,
): Record<string, unknown> | null {
  const root = recordValue(preview.geojson);
  if (!root) return null;
  if (root.type === "Polygon" || root.type === "MultiPolygon") return root;
  if (root.type === "Feature") return recordValue(root.geometry);
  if (root.type !== "FeatureCollection" || !Array.isArray(root.features)) return null;

  const descriptor = preview.features.find((feature) => feature.id === featureId);
  if (!descriptor) return null;
  const byFeatureId = root.features.find((candidate) => {
    const feature = recordValue(candidate);
    return feature !== null && String(feature.id ?? "") === descriptor.id;
  });
  const sourceIndex = Number.isSafeInteger(descriptor.source_index)
    ? descriptor.source_index as number
    : /^\d+$/.test(descriptor.id)
      ? Number(descriptor.id)
      : descriptor.index - 1;
  const feature = recordValue(byFeatureId ?? root.features[sourceIndex]);
  return feature?.type === "Feature" ? recordValue(feature.geometry) : null;
}

export function combineNisarSubsetBoundaryFeatures(
  preview: AoiPreviewOk,
  selectedIds: ReadonlySet<string>,
): NisarSubsetBoundaryGeometryResult {
  if (selectedIds.size === 0) {
    return { ok: false, error: "请至少选择一个可用的多边形要素。" };
  }
  const selectedDescriptors = preview.features.filter((feature) => selectedIds.has(feature.id));
  if (selectedDescriptors.length !== selectedIds.size) {
    return { ok: false, error: "所选边界要素与当前预览不一致，请重新选择。" };
  }

  const geometries: Record<string, unknown>[] = [];
  for (const descriptor of selectedDescriptors) {
    const geometry = previewFeatureGeometry(preview, descriptor.id);
    if (
      !geometry
      || (geometry.type !== "Polygon" && geometry.type !== "MultiPolygon")
      || !Array.isArray(geometry.coordinates)
    ) {
      return { ok: false, error: "所选要素不包含可用的 Polygon 或 MultiPolygon 几何。" };
    }
    geometries.push(geometry);
  }
  if (geometries.length === 0) {
    return { ok: false, error: "所选要素不包含可用的 Polygon 或 MultiPolygon 几何。" };
  }
  if (geometries.length === 1) {
    return { ok: true, geometry: structuredClone(geometries[0]!) };
  }

  const coordinates: unknown[] = [];
  for (const geometry of geometries) {
    const value = geometry.coordinates as unknown[];
    if (geometry.type === "Polygon") coordinates.push(structuredClone(value));
    else coordinates.push(...structuredClone(value));
  }
  return { ok: true, geometry: { type: "MultiPolygon", coordinates } };
}
