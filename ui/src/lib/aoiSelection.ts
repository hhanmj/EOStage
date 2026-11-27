import { canonicalAdministrativeAoiGeometry } from "./administrativeAoiGeometry.ts";

export type ChinaSelectorState = {
  province: string;
  city: string;
  district: string;
};

export type ChinaSelectorProviderIdentities = {
  province: string | null;
  city: string | null;
  district: string | null;
};

export type ChinaSelectorChange = {
  field: keyof ChinaSelectorState;
  value: string;
};

export type BoundaryBbox = {
  west: number;
  east: number;
  south: number;
  north: number;
  crs?: string;
};

export type BoundaryCandidate = {
  id?: string;
  label: string;
  name?: string;
  level?: string | null;
  boundary_id?: string;
  provider?: string;
  provider_object_id?: string;
  display_name?: string;
  admin_level?: string | null;
  geometry?: unknown;
  geojson?: unknown;
  bbox: BoundaryBbox;
  source?: string;
};

export type SelectorBoundaryTarget = ChinaSelectorState & {
  expectedName: string;
  expectedLevel: "country" | "province" | "city" | "district";
};

export type ActualAoi = {
  identity: string;
  source: string;
  level: string | null;
  displayName: string;
  geometry: unknown;
  bbox: BoundaryBbox | null;
};

export type AoiSelectionState = {
  chinaSelector: ChinaSelectorState;
  globalSearchQuery: string;
  globalSearchResults: BoundaryCandidate[];
  selectedSearchResult: BoundaryCandidate | null;
  previewBoundary: BoundaryCandidate | null;
  workingAoi: ActualAoi | null;
  persistedRegionAoi: ActualAoi | null;
};

export type AoiSelectionEvent =
  | { type: "selector/change"; change: ChinaSelectorChange }
  | { type: "global/query"; query: string }
  | { type: "global/results"; results: BoundaryCandidate[] }
  | { type: "global/select"; identity: string }
  | { type: "preview/boundary"; boundary: BoundaryCandidate | null }
  | { type: "bind/selected" }
  | { type: "working/restore"; aoi: ActualAoi | null }
  | { type: "region/restore"; aoi: ActualAoi | null };

const ALL_VALUES = new Set(["", "全部", "不限"]);
const NATIONAL_VALUES = new Set(["", "全国", "全部", "不限"]);
const DIRECT_CONTROLLED_MUNICIPALITY_ADCODES = new Set([
  "110000",
  "120000",
  "310000",
  "500000",
]);
const DIRECT_CONTROLLED_MUNICIPALITY_NAMES = new Set(["北京市", "天津市", "上海市", "重庆市"]);
const DATAV_DIRECT_SECOND_LEVEL_ADCODES = new Set(["810000", "820000"]);
const DATAV_DIRECT_SECOND_LEVEL_NAMES = new Set(["香港特别行政区", "澳门特别行政区"]);
const TAIWAN_PROVINCE_ADCODE = "710000";
const TAIWAN_PROVINCE_NAME = "台湾省";

export type ChinaSelectorPresentation = {
  municipality: boolean;
  twoLevel: boolean;
  secondLevel: "city" | "district";
  showThirdSelector: boolean;
};

function normalized(value: string | null | undefined): string {
  return String(value ?? "").trim().replace(/\s+/g, "").toLocaleLowerCase();
}

function isAll(value: string): boolean {
  return ALL_VALUES.has(value.trim());
}

function isNational(value: string): boolean {
  return NATIONAL_VALUES.has(value.trim());
}

function exactDataVAdcode(value: string | null | undefined): string | null {
  if (!value || value.trim() !== value || !/^\d{6}$/.test(value)) return null;
  return value;
}

function exactTiandituGbCode(value: string | null | undefined): string | null {
  if (!value || value.trim() !== value || !/^\d{9}$/.test(value)) return null;
  return value;
}

function exactDomesticTaiwanCode(value: string | null | undefined): string | null {
  return exactDataVAdcode(value) ?? exactTiandituGbCode(value);
}

export function isDirectControlledMunicipality(adcode: string | null | undefined): boolean {
  const exact = exactDataVAdcode(adcode);
  return exact !== null && DIRECT_CONTROLLED_MUNICIPALITY_ADCODES.has(exact);
}

export function isTaiwanSelectorProvince(
  providerObjectId: string | null | undefined,
  provinceName?: string | null,
): boolean {
  return exactDataVAdcode(providerObjectId) === TAIWAN_PROVINCE_ADCODE
    || String(provinceName ?? "").trim() === TAIWAN_PROVINCE_NAME;
}

export function chinaSelectorPresentation(
  provinceProviderObjectId: string | null | undefined,
  provinceName?: string | null,
): ChinaSelectorPresentation {
  const exactProvinceName = String(provinceName ?? "").trim();
  const municipality = isDirectControlledMunicipality(provinceProviderObjectId)
    || DIRECT_CONTROLLED_MUNICIPALITY_NAMES.has(exactProvinceName);
  const directDataVSecondLevel = DATAV_DIRECT_SECOND_LEVEL_ADCODES.has(
    exactDataVAdcode(provinceProviderObjectId) ?? "",
  ) || DATAV_DIRECT_SECOND_LEVEL_NAMES.has(exactProvinceName);
  const taiwan = isTaiwanSelectorProvince(provinceProviderObjectId, exactProvinceName);
  const twoLevel = municipality || directDataVSecondLevel || taiwan;
  return {
    municipality,
    twoLevel,
    secondLevel: municipality || directDataVSecondLevel ? "district" : "city",
    showThirdSelector: !twoLevel,
  };
}

export function selectedChinaProviderObjectId(
  state: ChinaSelectorState,
  identities: ChinaSelectorProviderIdentities,
): string | null {
  if (isTaiwanSelectorProvince(identities.province, state.province)) {
    return exactDomesticTaiwanCode(identities.city)
      ?? exactDomesticTaiwanCode(identities.province);
  }
  if (isNational(state.province)) return exactDataVAdcode(identities.province);
  if (!isAll(state.district)) return exactDataVAdcode(identities.district);
  if (!isAll(state.city)) return exactDataVAdcode(identities.city);
  return exactDataVAdcode(identities.province);
}

function boundaryGeometry(boundary: BoundaryCandidate): unknown | null {
  const geometry = boundary.geometry ?? boundary.geojson ?? null;
  return canonicalAdministrativeAoiGeometry(geometry);
}

function boundaryName(boundary: BoundaryCandidate): string {
  return boundary.name?.trim()
    || boundary.display_name?.split("/").at(-1)?.trim()
    || boundary.label.trim();
}

function boundaryLevel(boundary: BoundaryCandidate): string {
  return normalized(boundary.admin_level || boundary.level);
}

export function reduceChinaSelector(
  state: ChinaSelectorState,
  change: ChinaSelectorChange,
): ChinaSelectorState {
  if (change.field === "province") {
    return { province: change.value, city: "全部", district: "全部" };
  }
  if (change.field === "city") {
    return { ...state, city: change.value, district: "全部" };
  }
  return { ...state, district: change.value };
}

export function selectorBoundaryTarget(
  state: ChinaSelectorState,
  identities?: ChinaSelectorProviderIdentities,
): SelectorBoundaryTarget | null {
  if (isNational(state.province)) {
    return {
      ...state,
      expectedName: "中国",
      expectedLevel: "country",
    };
  }
  if (!isAll(state.district)) {
    return {
      ...state,
      expectedName: state.district.trim(),
      expectedLevel: "district",
    };
  }
  if (!isAll(state.city)) {
    const presentation = chinaSelectorPresentation(identities?.province, state.province);
    return {
      ...state,
      expectedName: state.city.trim(),
      expectedLevel: presentation.secondLevel,
    };
  }
  return {
    ...state,
    expectedName: state.province.trim(),
    expectedLevel: "province",
  };
}

export function stableBoundaryIdentity(boundary: BoundaryCandidate): string {
  const boundaryId = boundary.boundary_id?.trim();
  if (boundaryId) return boundaryId;
  const provider = boundary.provider?.trim();
  const providerObjectId = boundary.provider_object_id?.trim();
  if (provider && providerObjectId) return `${provider}:${providerObjectId}`;
  return boundary.id?.trim() || "";
}

export function findExactSelectorBoundary<T extends BoundaryCandidate>(
  results: readonly T[],
  target: SelectorBoundaryTarget | null,
): T | null {
  if (!target) return null;
  const expectedName = normalized(target.expectedName);
  const expectedLevel = normalized(target.expectedLevel);
  return results.find((boundary) =>
    Boolean(stableBoundaryIdentity(boundary))
    && boundaryLevel(boundary) === expectedLevel
    && normalized(boundaryName(boundary)) === expectedName) ?? null;
}

export type ChinaSelectorBoundaryPlan<T extends BoundaryCandidate> =
  | { kind: "bind"; boundary: T }
  | { kind: "choose_candidates"; candidates: readonly T[] }
  | { kind: "not_found" };

export function planChinaSelectorBoundaryResult<T extends BoundaryCandidate>(
  provider: string | null | undefined,
  results: readonly T[],
  target: SelectorBoundaryTarget | null,
  options: { directResolvedSelector?: boolean } = {},
): ChinaSelectorBoundaryPlan<T> {
  if (options.directResolvedSelector) {
    const resolved = results.filter(
      (boundary) =>
        (normalized(boundary.provider) === "datav" || normalized(boundary.provider) === "tianditu")
        && Boolean(stableBoundaryIdentity(boundary))
        && boundaryGeometry(boundary) !== null,
    );
    return resolved.length === 1
      ? { kind: "bind", boundary: resolved[0] }
      : { kind: "not_found" };
  }
  const normalizedProvider = normalized(provider);
  const nominatimCandidates = results.filter(
    (boundary) => normalized(boundary.provider) === "nominatim" && Boolean(stableBoundaryIdentity(boundary)),
  );
  if (normalizedProvider === "nominatim" || nominatimCandidates.length > 0) {
    return nominatimCandidates.length > 0
      ? { kind: "choose_candidates", candidates: nominatimCandidates }
      : { kind: "not_found" };
  }
  const exact = findExactSelectorBoundary(results, target);
  return exact ? { kind: "bind", boundary: exact } : { kind: "not_found" };
}

export function boundaryToActualAoi(boundary: BoundaryCandidate): ActualAoi | null {
  const identity = stableBoundaryIdentity(boundary);
  const geometry = boundaryGeometry(boundary);
  if (!identity || !geometry) return null;
  return {
    identity,
    source: boundary.provider?.trim() || boundary.source?.trim() || "administrative_boundary",
    level: boundary.admin_level?.trim() || boundary.level?.trim() || null,
    displayName: boundary.display_name?.trim() || boundary.label.trim(),
    geometry,
    bbox: boundary.bbox ?? null,
  };
}

export function createAoiSelectionState(
  initial: Partial<AoiSelectionState> = {},
): AoiSelectionState {
  return {
    chinaSelector: initial.chinaSelector ?? {
      province: "全国",
      city: "全部",
      district: "全部",
    },
    globalSearchQuery: initial.globalSearchQuery ?? "",
    globalSearchResults: initial.globalSearchResults ?? [],
    selectedSearchResult: initial.selectedSearchResult ?? null,
    previewBoundary: initial.previewBoundary ?? null,
    workingAoi: initial.workingAoi ?? null,
    persistedRegionAoi: initial.persistedRegionAoi ?? null,
  };
}

export function reduceAoiSelection(
  state: AoiSelectionState,
  event: AoiSelectionEvent,
): AoiSelectionState {
  switch (event.type) {
    case "selector/change":
      return {
        ...state,
        chinaSelector: reduceChinaSelector(state.chinaSelector, event.change),
      };
    case "global/query":
      return { ...state, globalSearchQuery: event.query };
    case "global/results":
      return {
        ...state,
        globalSearchResults: [...event.results],
        selectedSearchResult: null,
        previewBoundary: null,
      };
    case "global/select": {
      const selected = state.globalSearchResults.find(
        (boundary) => stableBoundaryIdentity(boundary) === event.identity,
      ) ?? null;
      return { ...state, selectedSearchResult: selected, previewBoundary: selected };
    }
    case "preview/boundary":
      return { ...state, previewBoundary: event.boundary };
    case "bind/selected": {
      const workingAoi = state.selectedSearchResult
        ? boundaryToActualAoi(state.selectedSearchResult)
        : null;
      return workingAoi ? { ...state, workingAoi } : state;
    }
    case "working/restore":
      return { ...state, workingAoi: event.aoi };
    case "region/restore":
      return { ...state, persistedRegionAoi: event.aoi };
  }
}

export function resolveActualAoi(input: {
  regionSelected: boolean;
  workingAoi: ActualAoi | null;
  persistedRegionAoi: ActualAoi | null;
  previewBoundary?: BoundaryCandidate | null;
}): ActualAoi | null {
  return input.regionSelected ? input.persistedRegionAoi : input.workingAoi;
}

export function aoiBboxInputValues(bbox: BoundaryBbox): {
  west: string;
  east: string;
  south: string;
  north: string;
} {
  return {
    west: String(bbox.west),
    east: String(bbox.east),
    south: String(bbox.south),
    north: String(bbox.north),
  };
}

export function beginLatestRequest(activeGeneration: number): {
  activeGeneration: number;
  requestGeneration: number;
} {
  const next = activeGeneration + 1;
  return { activeGeneration: next, requestGeneration: next };
}

export function isLatestResponse(
  activeGeneration: number,
  responseGeneration: number,
): boolean {
  return activeGeneration === responseGeneration;
}
