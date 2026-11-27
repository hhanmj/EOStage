import { t } from "../i18n/index.ts";

export type DemProviderId =
  | "COPERNICUS_AWS"
  | "FABDEM_PUBLIC"
  | "GEM15_PUBLIC"
  | "OPENTOPOGRAPHY"
  | "NISAR_DEM_ASF"
  | "EARTHDATA";

export type DemDatasetOption = {
  value: string;
  provider: DemProviderId;
  label: string;
  enabled: boolean;
  hint?: string;
};

export type DemProductOption = {
  value: string;
  label: string;
  enabled: boolean;
  hint?: string;
  providerPriority: readonly DemProviderId[];
};

export type DemRequestBbox = {
  west: number;
  east: number;
  south: number;
  north: number;
  crs?: string;
};

// DEM planning/display follows the exact AOI bounds (owner decision 2026-09-07).
export const DEM_BBOX_BUFFER_DEGREES = 0;
export const DEM_BBOX_DECIMALS = 3;

export const DEM_DATASET_GROUPS: {
  label: string;
  options: DemDatasetOption[];
}[] = [
  {
    get label() { return t("shared.demSources.groupAwsPublic"); },
    options: [
      {
        value: "COP30",
        provider: "COPERNICUS_AWS",
        label: "Copernicus Global DSM 30m",
        enabled: true,
        get hint() { return t("shared.demSources.hintEgm2008Resume"); },
      },
      {
        value: "COP90",
        provider: "COPERNICUS_AWS",
        label: "Copernicus Global DSM 90m",
        enabled: true,
        get hint() { return t("shared.demSources.hintEgm2008Resume"); },
      },
    ],
  },
  {
    get label() { return t("shared.demSources.groupBareEarth"); },
    options: [
      {
        value: "FABDEM",
        provider: "FABDEM_PUBLIC",
        label: "FABDEM V1-2 30m (去树冠/建筑裸地)",
        enabled: true,
        get hint() { return t("shared.demSources.hintFabdemNonCommercial"); },
      },
    ],
  },
  {
    get label() { return t("shared.demSources.groupGem15Public"); },
    options: [
      {
        value: "GEM15_DTM",
        provider: "GEM15_PUBLIC",
        label: "Google GEM-15 DTM 15m (裸地地形 · 海外)",
        enabled: true,
        get hint() { return t("shared.demSources.hintGem15Dtm"); },
      },
      {
        value: "GEM15_DSM",
        provider: "GEM15_PUBLIC",
        label: "Google GEM-15 DSM 15m (地表高程 · 海外)",
        enabled: true,
        get hint() { return t("shared.demSources.hintGem15Dsm"); },
      },
    ],
  },
  {
    get label() { return t("shared.demSources.groupOpenTopography"); },
    options: [
      { value: "COP30", provider: "OPENTOPOGRAPHY", label: "Copernicus Global DSM 30m", enabled: true, get hint() { return t("shared.demSources.hintEgm2008"); } },
      { value: "COP90", provider: "OPENTOPOGRAPHY", label: "Copernicus Global DSM 90m", enabled: true, get hint() { return t("shared.demSources.hintEgm2008"); } },
      { value: "NASADEM", provider: "OPENTOPOGRAPHY", label: "NASADEM 30m", enabled: true, get hint() { return t("shared.demSources.hintEgm96"); } },
      { value: "SRTM_GL3", provider: "OPENTOPOGRAPHY", label: "SRTM 90m", enabled: true, get hint() { return t("shared.demSources.hintEgm96"); } },
      { value: "SRTM_GL1", provider: "OPENTOPOGRAPHY", label: "SRTM 30m", enabled: true, get hint() { return t("shared.demSources.hintEgm96"); } },
      { value: "AW3D30", provider: "OPENTOPOGRAPHY", label: "ALOS World 3D 30m", enabled: true, get hint() { return t("shared.demSources.hintEgm96"); } },
      {
        value: "SRTM_GL1_ELLIPSOIDAL",
        provider: "OPENTOPOGRAPHY",
        label: "SRTM GL1 Ellipsoidal 30m",
        enabled: true,
        get hint() { return t("shared.demSources.hintWgs84NoConvert"); },
      },
      {
        value: "AW3D30_ELLIPSOIDAL",
        provider: "OPENTOPOGRAPHY",
        label: "ALOS World 3D Ellipsoidal 30m",
        enabled: true,
        get hint() { return t("shared.demSources.hintWgs84NoConvert"); },
      },
    ],
  },
  {
    get label() { return t("shared.demSources.groupAsfNisar"); },
    options: [
      {
        value: "NISAR_DEM",
        provider: "NISAR_DEM_ASF",
        label: "NISAR DEM 30m",
        enabled: true,
        get hint() { return t("shared.demSources.hintWgs84Resume"); },
      },
    ],
  },
  {
    get label() { return t("shared.demSources.groupEarthdata"); },
    options: [
      {
        value: "ASTER_GDEM_V3",
        provider: "EARTHDATA",
        label: "ASTER GDEM v3 (30m)",
        enabled: true,
        get hint() { return t("shared.demSources.hintEgm96"); },
      },
    ],
  },
];

const SOURCE_OPTIONS = DEM_DATASET_GROUPS.flatMap((group) => group.options);
const ROUTES_BY_DATASET = new Map<string, DemDatasetOption[]>();
for (const option of SOURCE_OPTIONS) {
  const routes = ROUTES_BY_DATASET.get(option.value) ?? [];
  routes.push(option);
  ROUTES_BY_DATASET.set(option.value, routes);
}

/**
 * User-facing DEM products. Provider duplication is deliberately removed here:
 * provider selection is an internal delivery decision, while product/resolution
 * remains the user's scientific choice.
 */
export const DEM_DATASET_OPTIONS: readonly DemProductOption[] = Array.from(
  ROUTES_BY_DATASET.entries(),
  ([value, routes]) => {
    const primary = routes[0]!;
    return Object.freeze({
      value,
      label: primary.label,
      enabled: routes.some((route) => route.enabled),
      get hint() { return primary.hint; },
      providerPriority: Object.freeze(routes.map((route) => route.provider)),
    });
  },
);

const PROVIDER_BY_DATASET = new Map<string, DemProviderId>();
for (const option of SOURCE_OPTIONS) {
  if (!PROVIDER_BY_DATASET.has(option.value)) {
    PROVIDER_BY_DATASET.set(option.value, option.provider);
  }
}

export const downloadableDemDatasets = new Set(PROVIDER_BY_DATASET.keys());

export function compatibleDemProviders(dataset: string): readonly DemProviderId[] {
  const datasetValue = dataset.trim().toUpperCase();
  return DEM_DATASET_OPTIONS.find((option) => option.value === datasetValue)?.providerPriority ?? [];
}

export function resolvedDemProvider(
  dataset: string,
  requestedProvider?: string | null,
): DemProviderId | null {
  const datasetValue = dataset.trim().toUpperCase();
  const providerValue = requestedProvider?.trim().toUpperCase();
  if (
    providerValue &&
    providerValue !== "AUTO" &&
    SOURCE_OPTIONS.some(
      (option) => option.value === datasetValue && option.provider === providerValue,
    )
  ) {
    return providerValue as DemProviderId;
  }
  return PROVIDER_BY_DATASET.get(datasetValue) ?? null;
}

export function demDatasetRequiresOpenTopographyKey(
  dataset: string,
  provider?: string | null,
): boolean {
  return resolvedDemProvider(dataset, provider) === "OPENTOPOGRAPHY";
}

export function demSourceOptionKey(option: DemDatasetOption): string {
  return `${option.provider}:${option.value}`;
}

export function expandedDemRequestBbox(
  bbox: DemRequestBbox,
  bufferDegrees = DEM_BBOX_BUFFER_DEGREES,
): DemRequestBbox {
  return {
    west: Math.max(-180, bbox.west - bufferDegrees),
    east: Math.min(180, bbox.east + bufferDegrees),
    south: Math.max(-90, bbox.south - bufferDegrees),
    north: Math.min(90, bbox.north + bufferDegrees),
    crs: bbox.crs || "EPSG:4326",
  };
}

export function formatDemBboxCoordinate(value: number): string {
  return value.toFixed(DEM_BBOX_DECIMALS);
}

export type FormalDemIdentity = {
  resource_id: string;
  capability_id: string;
  dataset: string;
  provider: string;
};

export function formalDemIdentity(dataset: string, requestedProvider = "auto"): FormalDemIdentity | null {
  const normalizedDataset = dataset.trim().toUpperCase();
  if (normalizedDataset === "USER_LOCAL") {
    return {
      resource_id: "dem.user-local.v1",
      capability_id: "capability.dem.user-local.convert.v1",
      dataset: normalizedDataset,
      provider: "LOCAL_FILE",
    };
  }
  const provider = resolvedDemProvider(normalizedDataset, requestedProvider);
  if (!provider) return null;
  const resourceByDataset: Record<string, string> = {
    COP30: "dem.cop30.v1",
    COP90: "dem.cop90.v1",
    FABDEM: "dem.fabdem.v1",
    GEM15_DTM: "dem.gem15-dtm.v1",
    GEM15_DSM: "dem.gem15-dsm.v1",
    NISAR_DEM: "dem.nisar-dem.v1",
    NASADEM: "dem.nasadem.v1",
    SRTM_GL3: "dem.srtm-gl3.v1",
    SRTM_GL1: "dem.srtm-gl1.v1",
    AW3D30: "dem.aw3d30.v1",
    SRTM_GL1_ELLIPSOIDAL: "dem.srtm-gl1-ellipsoidal.v1",
    AW3D30_ELLIPSOIDAL: "dem.aw3d30-ellipsoidal.v1",
    ASTER_GDEM_V3: "dem.aster-gdem.v3",
  };
  const resource_id = resourceByDataset[normalizedDataset];
  if (!resource_id) return null;
  const capability_id = normalizedDataset === "COP30" || normalizedDataset === "COP90"
    ? `capability.dem.${normalizedDataset.toLowerCase()}.${provider === "COPERNICUS_AWS" ? "aws" : "opentopography"}-download.v1`
    : normalizedDataset === "ASTER_GDEM_V3"
      ? "capability.dem.aster-gdem-v3.download.v1"
      : `capability.dem.${resource_id.slice("dem.".length, -".v1".length)}.download.v1`;
  return { resource_id, capability_id, dataset: normalizedDataset, provider };
}
