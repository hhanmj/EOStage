import { t } from "../i18n/index.ts";
import type {
  BBox,
  DatasetCapabilities,
  FilterDescriptor,
  SarBurstSelection,
  SarFilterRange,
  SarSearchQuery,
} from "./generated/v3-contracts.ts";

export type SarSearchFormRange = {
  start: string;
  end: string;
};

export type SarSearchFormState = {
  capabilities: DatasetCapabilities;
  dataset: string;
  platforms: string[];
  processingLevels: string[];
  beamModes: string[];
  polarizations: string[];
  flightDirections: string[];
  lookDirection: string | null;
  trackRange: SarSearchFormRange;
  frameRange: SarSearchFormRange;
  burstAbsoluteId: string;
  burstRelativeId: string;
  burstFullId: string;
};

export type SarSearchCommonInputs = {
  provider: string;
  start: string | null;
  end: string | null;
  bbox: BBox | null;
  aoiGeojson: Record<string, unknown> | null;
  aoiWkt?: string | null;
  maxResults: number | null;
  burst?: SarBurstSelection | null;
};

export type SarSearchQueryDraft = Omit<
  Partial<SarSearchQuery>,
  "dataset" | "bbox"
> & {
  dataset: string;
  bbox?: (Omit<BBox, "crs"> & { crs?: string }) | null;
};

const EMPTY_RANGE: SarSearchFormRange = { start: "", end: "" };

export function parseSarMaxResultsInput(value: string): number | null {
  const normalized = value.trim();
  if (!normalized) return 250;
  if (!/^\d+$/.test(normalized)) {
    throw new Error(t("shared.sarSearchForm.invalidResultLimit"));
  }
  const resultLimit = Number(normalized);
  if (!Number.isSafeInteger(resultLimit) || resultLimit < 1 || resultLimit > 10_000) {
    throw new Error(t("shared.sarSearchForm.invalidResultLimit"));
  }
  return resultLimit;
}

export function createSarSearchFormState(
  capabilities: DatasetCapabilities,
): SarSearchFormState {
  const sentinelDefaults =
    capabilities.dataset.trim().toUpperCase() === "SENTINEL-1" &&
    capabilities.processing_levels.includes("SLC")
      ? ["SLC"]
      : [];
  return {
    capabilities,
    dataset: capabilities.dataset,
    platforms: singletonDefault(capabilities.platforms),
    processingLevels: sentinelDefaults.length
      ? sentinelDefaults
      : singletonDefault(capabilities.processing_levels),
    beamModes: singletonDefault(capabilities.beam_modes),
    polarizations: singletonDefault(capabilities.polarizations),
    flightDirections: [],
    lookDirection: null,
    trackRange: { ...EMPTY_RANGE },
    frameRange: { ...EMPTY_RANGE },
    burstAbsoluteId: "",
    burstRelativeId: "",
    burstFullId: "",
  };
}

export function reconcileSarSearchForm(
  state: SarSearchFormState,
  capabilities: DatasetCapabilities,
): SarSearchFormState {
  const initial = createSarSearchFormState(capabilities);
  const datasetChanged = state.dataset !== capabilities.dataset;
  return {
    ...initial,
    platforms: datasetChanged
      ? initial.platforms
      : legalSelectionsOrSingleton(state.platforms, capabilities.platforms),
    processingLevels: datasetChanged
      ? initial.processingLevels
      : legalSelectionsOrSingleton(
          state.processingLevels,
          capabilities.processing_levels,
        ),
    beamModes: datasetChanged
      ? initial.beamModes
      : legalSelectionsOrSingleton(state.beamModes, capabilities.beam_modes),
    polarizations: datasetChanged
      ? initial.polarizations
      : legalSelectionsOrSingleton(
          state.polarizations,
          capabilities.polarizations,
        ),
    flightDirections: capabilities.supports_flight_direction
      ? unique(state.flightDirections)
      : [],
    lookDirection: capabilities.supports_look_direction
      ? state.lookDirection
      : null,
    trackRange: datasetChanged ? { ...EMPTY_RANGE } : { ...state.trackRange },
    frameRange: datasetChanged ? { ...EMPTY_RANGE } : { ...state.frameRange },
    burstAbsoluteId: datasetChanged ? "" : state.burstAbsoluteId,
    burstRelativeId: datasetChanged ? "" : state.burstRelativeId,
    burstFullId: datasetChanged ? "" : state.burstFullId,
  };
}

export function buildSarSearchQuery(
  state: SarSearchFormState,
  inputs: SarSearchCommonInputs,
): SarSearchQuery {
  const trackFilter = validatedFilterProjection(
    state.trackRange,
    state.capabilities.track_filter,
    "Track",
    state.platforms,
  );
  const frameFilter = validatedFilterProjection(
    state.frameRange,
    state.capabilities.frame_filter,
    "Frame",
    state.platforms,
  );
  return {
    provider: inputs.provider,
    dataset: state.dataset,
    platforms: [...state.platforms],
    instrument: null,
    processing_levels: [...state.processingLevels],
    product_families: [],
    beam_modes: [...state.beamModes],
    polarizations: [...state.polarizations],
    flight_direction: state.flightDirections.length === 1
      ? state.flightDirections[0]
      : null,
    look_direction: state.lookDirection,
    relative_orbit: integerScalar(trackFilter.scalar, state.capabilities.track_filter),
    frame: integerScalar(frameFilter.scalar, state.capabilities.frame_filter),
    asf_frame: paddedScalar(frameFilter.scalar, state.capabilities.frame_filter),
    path: null,
    start: inputs.start,
    end: inputs.end,
    track_range: trackFilter.range ?? paddedScalarRange(
      trackFilter.scalar,
      state.capabilities.track_filter,
    ),
    frame_range: frameFilter.range,
    aoi_geojson: inputs.aoiGeojson,
    aoi_wkt: inputs.aoiWkt ?? null,
    bbox: inputs.bbox,
    max_results: inputs.maxResults,
    burst: inputs.burst ?? burstSelection(state),
  };
}

function burstSelection(state: SarSearchFormState): SarBurstSelection | null {
  if (state.dataset.trim().toUpperCase() !== "SLC-BURST") return null;
  const fullBurstId = state.burstFullId.trim();
  const absoluteBurstId = optionalBurstInteger(state.burstAbsoluteId, "Absolute Burst ID");
  const relativeBurstId = optionalBurstInteger(state.burstRelativeId, "Relative Burst ID");
  if (!fullBurstId && absoluteBurstId === null && relativeBurstId === null) return null;
  return {
    burst_id: null,
    full_burst_id: fullBurstId || null,
    relative_burst_id: relativeBurstId,
    absolute_burst_id: absoluteBurstId,
    subswath: null,
    burst_index: null,
  };
}

function optionalBurstInteger(value: string, label: string): number | null {
  const normalized = value.trim();
  if (!normalized) return null;
  if (!/^\d+$/.test(normalized)) throw new Error(`${label} must be a non-negative integer`);
  return Number(normalized);
}

export function completeSarSearchQuery(
  draft: SarSearchQueryDraft,
): SarSearchQuery {
  return {
    provider: draft.provider || "ASF",
    dataset: draft.dataset,
    platforms: [...(draft.platforms ?? [])],
    instrument: draft.instrument ?? null,
    processing_levels: [...(draft.processing_levels ?? [])],
    product_families: [...(draft.product_families ?? [])],
    beam_modes: [...(draft.beam_modes ?? [])],
    polarizations: [...(draft.polarizations ?? [])],
    flight_direction: draft.flight_direction ?? null,
    look_direction: draft.look_direction ?? null,
    relative_orbit: draft.relative_orbit ?? null,
    frame: draft.frame ?? null,
    asf_frame: draft.asf_frame ?? null,
    path: draft.path ?? null,
    start: draft.start ?? null,
    end: draft.end ?? null,
    track_range: draft.track_range ? { ...draft.track_range } : null,
    frame_range: draft.frame_range ? { ...draft.frame_range } : null,
    aoi_geojson: draft.aoi_geojson ?? null,
    aoi_wkt: draft.aoi_wkt ?? null,
    bbox: draft.bbox
      ? { ...draft.bbox, crs: draft.bbox.crs ?? "EPSG:4326" }
      : null,
    max_results: draft.max_results ?? null,
    burst: draft.burst ?? null,
  };
}

type FilterProjection = {
  range: SarFilterRange | null;
  scalar: string | null;
};

function validatedFilterProjection(
  range: SarSearchFormRange,
  descriptor: FilterDescriptor | null,
  fallbackLabel: string,
  platforms: string[],
): FilterProjection {
  const start = range.start.trim();
  const end = range.end.trim();
  if (!start && !end) return { range: null, scalar: null };
  const label = descriptor?.label || fallbackLabel;
  if (!descriptor?.supported) {
    throw new Error(`${label} range is not supported for this dataset`);
  }
  for (const value of [start, end]) {
    if (value && !isValidRangeValue(value, descriptor, platforms)) {
      throw new Error(`${label} endpoint "${value}" is invalid`);
    }
  }
  if (!start || !end) return { range: null, scalar: start || end };
  if (numericStringGreater(start, end)) {
    throw new Error(`${label} range start cannot be greater than end`);
  }
  return { range: { start, end }, scalar: null };
}

function integerScalar(
  value: string | null,
  descriptor: FilterDescriptor | null,
): number | null {
  return value != null && descriptor?.value_type === "integer" ? Number(value) : null;
}

function paddedScalar(
  value: string | null,
  descriptor: FilterDescriptor | null,
): string | null {
  return value != null && descriptor?.value_type === "padded_string" ? value : null;
}

function paddedScalarRange(
  value: string | null,
  descriptor: FilterDescriptor | null,
): SarFilterRange | null {
  return value != null && descriptor?.value_type === "padded_string"
    ? { start: value, end: value }
    : null;
}

function isValidRangeValue(
  value: string,
  descriptor: FilterDescriptor,
  platforms: string[],
): boolean {
  if (descriptor.pattern && !new RegExp(descriptor.pattern).test(value)) return false;
  if (descriptor.exact_width != null && value.length !== descriptor.exact_width) return false;
  const requiresNumericValue = descriptor.value_type === "integer"
    || descriptor.minimum != null
    || descriptor.maximum != null
    || (descriptor.platform_limits?.length ?? 0) > 0;
  if (!requiresNumericValue) return true;
  if (!/^\d+$/.test(value)) return false;
  const numeric = Number(value);
  if (!Number.isSafeInteger(numeric)) return false;
  const [minimum, maximum] = effectiveFilterBounds(descriptor, platforms);
  if (minimum != null && numeric < minimum) return false;
  if (maximum != null && numeric > maximum) return false;
  return minimum == null || maximum == null || minimum <= maximum;
}

function effectiveFilterBounds(
  descriptor: FilterDescriptor,
  platforms: string[],
): [number | null, number | null] {
  let minimum = descriptor.minimum ?? null;
  let maximum = descriptor.maximum ?? null;
  for (const platform of unique(platforms)) {
    const limit = descriptor.platform_limits?.find(
      (candidate) => candidate.platform.toUpperCase() === platform.toUpperCase(),
    );
    if (!limit) continue;
    minimum = minimum == null ? limit.minimum : Math.max(minimum, limit.minimum);
    maximum = maximum == null ? limit.maximum : Math.min(maximum, limit.maximum);
  }
  return [minimum, maximum];
}

function numericStringGreater(left: string, right: string): boolean {
  const normalizedLeft = left.replace(/^0+/, "") || "0";
  const normalizedRight = right.replace(/^0+/, "") || "0";
  return normalizedLeft.length > normalizedRight.length
    || (normalizedLeft.length === normalizedRight.length && normalizedLeft > normalizedRight);
}

function legalSelections(selected: string[], allowed: string[]): string[] {
  const legal = new Set(allowed);
  return unique(selected).filter((value) => legal.has(value));
}

function singletonDefault(allowed: string[]): string[] {
  return allowed.length === 1 ? [allowed[0]] : [];
}

function legalSelectionsOrSingleton(selected: string[], allowed: string[]): string[] {
  const legal = legalSelections(selected, allowed);
  return legal.length ? legal : singletonDefault(allowed);
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}
