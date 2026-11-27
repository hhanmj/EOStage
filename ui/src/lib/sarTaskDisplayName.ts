import { t } from "../i18n/index.ts";

type SarTaskSceneNameSource = {
  dataset?: string | null;
  provider?: string | null;
  sar_product?: {
    dataset?: string | null;
    provider?: string | null;
  } | null;
};

export type SarTaskNameSource = {
  name?: string | null;
  dataset_id?: string | null;
  task_type?: string | null;
  approved_worker_operation?: string | null;
  snapshot_scenes?: readonly SarTaskSceneNameSource[] | null;
  snapshot?: readonly SarTaskSceneNameSource[] | null;
  batch_context?: { query?: unknown } | null;
  batch_id?: string | null;
  provider_id?: string | null;
  operation?: { source_provider?: string | null } | null;
};

export type TaskDeliveryProviderPresentation = {
  id: string;
  label: string;
  officialUrl: string | null;
};

const DELIVERY_PROVIDERS: Readonly<Record<string, Omit<TaskDeliveryProviderPresentation, "id">>> = {
  "asf.sentinel1": { label: "ASF", officialUrl: "https://asf.alaska.edu/" },
  "asf.nasa.earthdata": { label: "ASF", officialUrl: "https://asf.alaska.edu/" },
  "asf.catalog": { label: "ASF", officialUrl: "https://asf.alaska.edu/" },
  "copernicus.dataspace.odata": { label: "CDSE", officialUrl: "https://dataspace.copernicus.eu/" },
  "microsoft.planetary-computer": { label: "MPC", officialUrl: "https://planetarycomputer.microsoft.com/" },
  "microsoft.planetary-computer.hls": { label: "MPC", officialUrl: "https://planetarycomputer.microsoft.com/" },
  "element84.earth-search": { label: "Earth Search", officialUrl: "https://earth-search.aws.element84.com/v1/" },
  "nasa.laads.earthdata": { label: "NASA LAADS", officialUrl: "https://ladsweb.modaps.eosdis.nasa.gov/" },
  "nasa.lpdaac.earthdata": { label: "NASA LP DAAC", officialUrl: "https://lpdaac.usgs.gov/" },
  "opera.lpdaac.earthdata": { label: "NASA LP DAAC", officialUrl: "https://lpdaac.usgs.gov/" },
  "dlr.enmap.stac": { label: "DLR EOC", officialUrl: "https://eoweb.dlr.de/" },
  "usgs.eros.m2m": { label: "USGS", officialUrl: "https://earthexplorer.usgs.gov/" },
  "copernicus.aws": { label: "AWS", officialUrl: "https://registry.opendata.aws/copernicus-dem/" },
  COPERNICUS_AWS: { label: "AWS", officialUrl: "https://registry.opendata.aws/copernicus-dem/" },
  OPENTOPOGRAPHY: { label: "OTG", officialUrl: "https://portal.opentopography.org/" },
  opentopography: { label: "OTG", officialUrl: "https://portal.opentopography.org/" },
  "opentopography.dem": { label: "OTG", officialUrl: "https://portal.opentopography.org/" },
  "bhoonidhi.nrsc": { label: "Bhoonidhi", officialUrl: "https://bhoonidhi.nrsc.gov.in/" },
  "isro.nrsc.bhoonidhi": { label: "Bhoonidhi", officialUrl: "https://bhoonidhi.nrsc.gov.in/" },
  "gacos.webform": { label: "GACOS", officialUrl: "https://www.gacos.net/" },
  "gacos.manual-result": { label: "GACOS", officialUrl: "https://www.gacos.net/" },
  "gacos.local-result": { label: "GACOS", officialUrl: "https://www.gacos.net/" },
  "sentinel1.orbit": { label: "CDSE Orbit", officialUrl: "https://dataspace.copernicus.eu/" },
};

const REVIEWED_DATASET_NAMES: Readonly<Record<string, string>> = {
  "SENTINEL-1": "Sentinel-1",
  "SLC-BURST": "Sentinel-1 SLC Burst",
  "SENTINEL-1 SLC BURST": "Sentinel-1 SLC Burst",
  "ALOS PALSAR": "ALOS PALSAR",
  get ERS() { return t("shared.sarTaskDisplay.ersRetired"); },
  get "JERS-1"() { return t("shared.sarTaskDisplay.jers1Retired"); },
  get "RADARSAT-1"() { return t("shared.sarTaskDisplay.radarsat1Retired"); },
  SEASAT: "SEASAT",
  "SAR.NISAR.L.RSLC.PROVISIONAL.V1": "NISAR RSLC",
  "SAR.NISAR.S.SSAR.RSLC.V1": "NISAR S-band SSAR RSLC",
  "SAR.ALOS-2.PALSAR-2.SCANSAR.L1.1.V1": "ALOS-2 PALSAR-2 ScanSAR L1.1",
  "OPTICAL.SENTINEL-2.L2A": "Sentinel-2 L2A",
  "OPTICAL.SENTINEL-2.L1C": "Sentinel-2 L1C",
  "OPTICAL.LANDSAT.C2.L2": "Landsat C2 L2",
  "OPTICAL.HLS.L30.V2": "HLS L30",
  "OPTICAL.HLS.S30.V2": "HLS S30",
};

function isRemoteDerivedSubset(source: SarTaskNameSource): boolean {
  const query = source.batch_context?.query;
  return Boolean(
    query &&
      typeof query === "object" &&
      !Array.isArray(query) &&
      (query as Record<string, unknown>).delivery_kind === "remote_derived_subset",
  );
}

function datasetDisplayName(dataset: string): string {
  const canonical = dataset.trim();
  return REVIEWED_DATASET_NAMES[canonical.toUpperCase()] ?? canonical;
}

function queryDatasets(batchContext: SarTaskNameSource["batch_context"]): string[] {
  const query = batchContext?.query;
  if (!query || typeof query !== "object" || Array.isArray(query)) return [];
  const record = query as Record<string, unknown>;
  if (typeof record.dataset === "string") return [record.dataset];
  if (!Array.isArray(record.datasets)) return [];
  return record.datasets.filter((value): value is string => typeof value === "string");
}

function frozenDatasetNames(source: SarTaskNameSource): string[] {
  const scenes = [
    ...(source.snapshot_scenes ?? []),
    ...(source.snapshot ?? []),
  ];
  const rawDatasets = scenes
    .map((scene) => scene.sar_product?.dataset?.trim() || scene.dataset?.trim() || "")
    .filter(Boolean);
  if (source.dataset_id?.trim()) rawDatasets.unshift(source.dataset_id.trim());
  if (!rawDatasets.length) rawDatasets.push(...queryDatasets(source.batch_context));

  const names = new Map<string, string>();
  for (const dataset of rawDatasets) {
    const canonical = dataset.trim();
    if (!canonical) continue;
    names.set(canonical.toUpperCase(), datasetDisplayName(canonical));
  }
  return Array.from(names.values());
}

function isLegacySentinelName(name: string): boolean {
  return /^(?:ASF\s+)?Sentinel-1(?:\s+download task|\s+数据下载|\s+下载任务)?$/i.test(name); // i18n-exempt: matches legacy stored task name
}

function isProviderImplementationName(name: string): boolean {
  return /^(?:ASF\s+SAR(?:\s+product)?(?:\s+download task|\s+数据下载)?|asf\.catalog)$/i.test(name); // i18n-exempt: matches legacy stored task name
}

type GacosTaskKind = "request" | "local_result" | "result";

function gacosTaskKind(source: SarTaskNameSource): GacosTaskKind | null {
  const operation = source.approved_worker_operation?.trim().toLowerCase();
  if (operation === "gacos_manual_request") return "request";
  if (operation === "gacos_import_result") return "local_result";
  if (operation === "gacos_fetch_result") return "result";

  const providerId = (source.provider_id || source.operation?.source_provider || "").trim().toLowerCase();
  if (providerId === "gacos.webform") return "request";
  if (providerId === "gacos.local-result") return "local_result";
  if (providerId === "gacos.manual-result") return "result";

  const taskType = source.task_type?.trim().toLowerCase() ?? "";
  const dataset = source.dataset_id?.trim().toLowerCase() ?? "";
  if (dataset === "gacos.atmosphere" || taskType === "gacos" || taskType.startsWith("gacos ")) {
    if (taskType.includes("result") || taskType.includes("结果")) return "result"; // i18n-exempt: matches backend task_type value
    return "request";
  }
  return null;
}

export function formatTaskTimestamp(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === "") return "—";
  const numeric = typeof value === "number" ? value : Number(value);
  const date = Number.isFinite(numeric) && numeric > 0
    ? new Date(numeric < 10_000_000_000 ? numeric * 1_000 : numeric)
    : new Date(String(value));
  if (Number.isNaN(date.getTime())) return "—";
  const pad = (part: number) => String(part).padStart(2, "0");
  return [date.getFullYear(), pad(date.getMonth() + 1), pad(date.getDate())].join("-")
    + " " + [pad(date.getHours()), pad(date.getMinutes()), pad(date.getSeconds())].join(":");
}

export function sarTaskTypeLabel(source: SarTaskNameSource): string {
  const gacos = gacosTaskKind(source);
  if (gacos === "result") return t("shared.sarTaskDisplay.gacosResultDownload");
  if (gacos === "local_result") return t("shared.sarTaskDisplay.gacosLocalImport");
  if (gacos === "request") return "GACOS request";
  const taskType = source.task_type?.trim().toLowerCase();
  if (taskType === "nisar_subset" || taskType === "remote_derived_subset_v2") {
    return t("shared.sarTaskDisplay.nisarAoiSubsetType");
  }
  if (taskType === "orbit") return t("shared.sarTaskDisplay.s1PreciseOrbitType");
  if (taskType === "nisar_complete") return t("shared.sarTaskDisplay.nisarCompleteType");
  if (source.batch_id?.startsWith("sentinel2-bands-")) return t("shared.sarTaskDisplay.s2BandsType");
  const datasetId = source.dataset_id?.trim().toUpperCase() ?? "";
  if (datasetId === "SENTINEL-1") return "Sentinel-1 SLC";
  if (datasetId === "SLC-BURST" || datasetId === "SENTINEL-1 SLC BURST") {
    return "Sentinel-1 SLC Burst";
  }
  if (datasetId === "OPTICAL.SENTINEL-2.L2A") return "Sentinel-2 L2A";
  if (datasetId === "OPTICAL.LANDSAT.C2.L2") return "Landsat C2 L2";
  const datasets = [
    source.dataset_id?.trim(),
    ...(source.snapshot_scenes ?? []).map((scene) => scene.sar_product?.dataset?.trim() || scene.dataset?.trim()),
    ...(source.snapshot ?? []).map((scene) => scene.sar_product?.dataset?.trim() || scene.dataset?.trim()),
    ...queryDatasets(source.batch_context),
  ].filter((value): value is string => Boolean(value));
  if (datasets.some((dataset) => dataset.toUpperCase() === "SAR.NISAR.L.RSLC.PROVISIONAL.V1")) {
    return t("shared.sarTaskDisplay.nisarCompleteType");
  }
  const reviewed = frozenDatasetNames(source);
  if (reviewed.length === 1) return reviewed[0];
  return "—";
}

function frozenProviderId(source: SarTaskNameSource): string {
  const providers = new Set(
    [...(source.snapshot_scenes ?? []), ...(source.snapshot ?? [])]
      .map((scene) => scene.sar_product?.provider?.trim() || scene.provider?.trim() || "")
      .filter(Boolean),
  );
  return providers.size === 1 ? [...providers][0]! : "";
}

export function taskDeliveryProvider(source: SarTaskNameSource): TaskDeliveryProviderPresentation {
  const batchId = source.batch_id?.trim() ?? "";
  let providerId = "";
  if (batchId.startsWith("landsat-assets-")) providerId = "microsoft.planetary-computer";
  else if (batchId.startsWith("landsat-delivery-")) providerId = "usgs.eros.m2m";
  else if (batchId.startsWith("sentinel2-archive-")) providerId = "copernicus.dataspace.odata";
  else if (batchId.startsWith("sentinel2-bands-")) providerId = "microsoft.planetary-computer";
  else if (batchId.startsWith("hls-bands-")) providerId = "microsoft.planetary-computer";
  else if (batchId.startsWith("optderived-")) providerId = "microsoft.planetary-computer";
  else providerId = source.operation?.source_provider?.trim()
    || source.provider_id?.trim()
    || frozenProviderId(source);
  const reviewed = DELIVERY_PROVIDERS[providerId];
  return reviewed
    ? { id: providerId, ...reviewed }
    : { id: providerId || "unknown", label: providerId || "—", officialUrl: null };
}

export function sarTaskDatasetLabel(source: SarTaskNameSource): string {
  if (source.task_type === "orbit") return "Sentinel-1 Orbit";
  if (gacosTaskKind(source)) {
    return "GACOS";
  }
  if (isRemoteDerivedSubset(source)) return t("shared.sarTaskDisplay.nisarDerivedDatasetLabel");
  const datasetNames = frozenDatasetNames(source);
  if (datasetNames.length) return datasetNames.join(" + ");
  const fallback = source.name?.trim() ?? "";
  if (isLegacySentinelName(fallback)) return "Sentinel-1";
  return "SAR";
}

export function sarTaskDisplayName(source: SarTaskNameSource): string {
  if (source.task_type === "orbit") return t("shared.sarTaskDisplay.s1PreciseOrbitTask");
  const gacos = gacosTaskKind(source);
  if (gacos === "result") return t("shared.sarTaskDisplay.gacosResultDownload");
  if (gacos === "local_result") return t("shared.sarTaskDisplay.gacosLocalImport");
  if (gacos === "request") return t("shared.sarTaskDisplay.gacosRequest");
  if (isRemoteDerivedSubset(source)) return t("shared.sarTaskDisplay.nisarRemoteDerivedTask");
  if (source.batch_id?.startsWith("sentinel2-bands-")) return t("shared.sarTaskDisplay.s2BandsDownloadTask");
  const datasetNames = frozenDatasetNames(source);
  if (datasetNames.length) return t("shared.sarTaskDisplay.datasetDownloadTask", { datasets: datasetNames.join(" + ") });

  const fallback = source.name?.trim() ?? "";
  if (isLegacySentinelName(fallback)) return t("shared.sarTaskDisplay.s1DownloadTask");
  if (!fallback || isProviderImplementationName(fallback)) return t("shared.sarTaskDisplay.sarDownloadTask");
  return fallback;
}
