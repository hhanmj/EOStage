import { t } from "../i18n/index.ts";
import type {
  CatalogSearchQuery,
  DatasetCapabilities,
  SarSourceCapability,
} from "./generated/v3-contracts.ts";
import type { DesktopEffectiveCapabilityStatus } from "./desktopShellBridge.ts";
import {
  BHOONIDHI_COLLECTION_ID,
  BHOONIDHI_LOGICAL_DATASET,
  BHOONIDHI_PROFILE_ID,
  BHOONIDHI_PROVIDER_ID,
} from "./bhoonidhiSsarCatalog.ts";

export const ALOS2_SOURCE_SELECTOR = "ALOS-2";
export const NISAR_SOURCE_SELECTOR = "NISAR";

export type NisarBand = "L" | "S";

type JsonObject = Record<string, unknown>;

export type TypedSarSearchInput = {
  source: SarSourceCapability;
  aoiGeojson: JsonObject | null;
  startUtc: string;
  endUtc: string;
  resultLimit: number | null;
  alos2BeamMode?: string | null;
  alos2FlightDirection?: string | null;
  alos2LookDirection?: string | null;
  alos2ProductType?: string | null;
  alos2ProcessingLevel?: string | null;
  nisarFrameCoverage?: string | null;
  nisarFrequencyA?: string | null;
  nisarFrequencyB?: string | null;
  nisarRangeBandwidth?: string | null;
  nisarFlightDirection?: string | null;
  nisarLookDirection?: string | null;
  nisarProductType?: string | null;
  nisarBand?: string | null;
  nisarMaturity?: string | null;
  bhoonidhiMode?: string | null;
  bhoonidhiPolarization?: string | null;
  bhoonidhiAcquisitionMode?: string | null;
  bhoonidhiSoftwareVersion?: string | null;
};

export type TypedSarSearchBuildResult =
  | { ok: true; query: CatalogSearchQuery }
  | { ok: false; error: string };

function record(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() === value && value.length > 0 ? value : null;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => stringValue(item) !== null)
    : [];
}

function strictStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => stringValue(item) !== null);
}

function projectedLegacyDatasetCapability(
  source: SarSourceCapability,
): DatasetCapabilities | null {
  const capability = record(source.filters.dataset_capabilities);
  if (!capability) return null;
  const arrayKeys = [
    "platforms",
    "instruments",
    "processing_levels",
    "product_families",
    "beam_modes",
    "polarizations",
  ] as const;
  const booleanKeys = [
    "supports_flight_direction",
    "supports_look_direction",
    "supports_relative_orbit",
    "supports_frame",
    "supports_path",
    "supports_burst",
  ] as const;
  if (
    stringValue(capability.dataset) !== source.selector
    || stringValue(capability.display_name) === null
    || arrayKeys.some((key) => !strictStringArray(capability[key]))
    || booleanKeys.some((key) => typeof capability[key] !== "boolean")
    || (capability.frame_filter !== null && record(capability.frame_filter) === null)
    || (capability.track_filter !== null && record(capability.track_filter) === null)
    || !Array.isArray(capability.supported_actions)
    || capability.supported_actions.some((value) => stringValue(record(value)?.action) === null)
  ) return null;
  return capability as DatasetCapabilities;
}

export type SarDatasetCapabilityProjection =
  | { ok: true; capability: DatasetCapabilities }
  | { ok: false; error: string };

function sourceOptions(source: SarSourceCapability): JsonObject {
  return record(source.filters.options) ?? {};
}

function sourceFixed(source: SarSourceCapability): JsonObject {
  return record(source.filters.fixed) ?? {};
}

export function nisarSourceForBand(
  sources: readonly SarSourceCapability[],
  band: NisarBand,
): SarSourceCapability | null {
  return sources.find((source) => (
    source.search_kind === "typed_catalog_profile"
    && sourceFixed(source).platform === "NISAR"
    && sourceFixed(source).band === band
    && source.logical_dataset?.startsWith("sar.nisar.") === true
  )) ?? null;
}

export type NisarSearchSourceAdmission =
  | { ok: true; source: SarSourceCapability }
  | { ok: false; error: string };

function sameNisarSourceAuthority(
  expected: SarSourceCapability,
  selected: SarSourceCapability,
): boolean {
  return expected.selector === selected.selector
    && expected.provider_id === selected.provider_id
    && expected.logical_dataset === selected.logical_dataset
    && expected.native_collection_id === selected.native_collection_id
    && expected.collection_profile_id === selected.collection_profile_id;
}

export function nisarSearchSourceAdmission(
  sources: readonly SarSourceCapability[],
  band: NisarBand,
  selectedSource: SarSourceCapability | null,
): NisarSearchSourceAdmission {
  const expected = nisarSourceForBand(sources, band);
  if (!expected) {
    return { ok: false, error: t("shared.sarCapabilities.nisarBandOnboarding", { band }) };
  }
  if (!selectedSource || !sameNisarSourceAuthority(expected, selectedSource)) {
    return { ok: false, error: t("shared.sarCapabilities.nisarBandMismatch") };
  }
  if (!expected.available) {
    return { ok: false, error: t("shared.sarCapabilities.nisarBandUnavailable", { band }) };
  }
  return { ok: true, source: expected };
}

export function nisarSubsetCapabilityMessage(
  status: DesktopEffectiveCapabilityStatus | null,
  band: NisarBand,
): string {
  if (!status) return t("shared.sarCapabilities.subsetChecking");
  switch (status.projection.reason) {
    case "ready":
      return t("shared.sarCapabilities.subsetReady");
    case "credential_missing_or_unknown":
      return band === "L"
        ? t("shared.sarCapabilities.subsetNeedEarthdata")
        : t("shared.sarCapabilities.subsetNeedBhoonidhi");
    case "admission_missing_rejected_or_unknown":
      return t("shared.sarCapabilities.subsetUnaccepted");
    case "build_unavailable_or_unknown":
      return t("shared.sarCapabilities.subsetMissingBinary");
    case "runtime_not_ready_or_unknown":
      return t("shared.sarCapabilities.subsetServiceNotReady");
    case "policy_denied_or_unknown":
      return t("shared.sarCapabilities.subsetConfigDisallowed");
    case "static_reserved":
      return t("shared.sarCapabilities.subsetOnboarding");
    case "static_retired":
      return t("shared.sarCapabilities.subsetRetired");
  }
}

export function sarSourceOptionValues(source: SarSourceCapability, key: string): string[] {
  return stringArray(sourceOptions(source)[key]);
}

/** Legal product-identity values, preferring a registry option set over one fixed value. */
export function sarSourceChoiceValues(source: SarSourceCapability, key: string): string[] {
  const options = sarSourceOptionValues(source, key);
  if (options.length) return [...new Set(options)];
  const fixed = sarSourceFixedValue(source, key);
  if (!fixed && source.selector === ALOS2_SOURCE_SELECTOR && key === "product_type") {
    const instrument = sarSourceFixedValue(source, "instrument");
    return instrument ? [`${instrument} ScanSAR`] : [];
  }
  return fixed ? [fixed] : [];
}

export function sarSourceFixedValue(source: SarSourceCapability, key: string): string | null {
  return stringValue(sourceFixed(source)[key]);
}

function selectedExecutableFixedChoice(
  source: SarSourceCapability,
  key: string,
  requested: string | null | undefined,
): { ok: true; value: string } | { ok: false; error: string } {
  const choices = sarSourceChoiceValues(source, key);
  const fixed = fixedValue(source, key);
  const executableFixed = fixed ?? (
    source.selector === ALOS2_SOURCE_SELECTOR && key === "product_type"
      ? sarSourceChoiceValues(source, key)[0] ?? null
      : null
  );
  const value = requested?.trim() || executableFixed || choices[0] || null;
  if (!value || !choices.includes(value)) {
    return { ok: false, error: t("shared.sarCapabilities.illegalProductCondition", { selector: source.selector, key }) };
  }
  if (!executableFixed || value !== executableFixed) {
    return { ok: false, error: t("shared.sarCapabilities.unmappedConditionValue", { selector: source.selector, key, value }) };
  }
  return { ok: true, value };
}

function optionValues(source: SarSourceCapability, key: string): string[] {
  return sarSourceOptionValues(source, key);
}

function fixedValue(source: SarSourceCapability, key: string): string | null {
  return sarSourceFixedValue(source, key);
}

export function isBhoonidhiSsarSource(source: SarSourceCapability): boolean {
  return source.provider_id === BHOONIDHI_PROVIDER_ID
    && source.logical_dataset === BHOONIDHI_LOGICAL_DATASET
    && source.native_collection_id === BHOONIDHI_COLLECTION_ID
    && source.collection_profile_id === BHOONIDHI_PROFILE_ID
    && fixedValue(source, "platform") === "NISAR"
    && fixedValue(source, "instrument") === "SSAR"
    && fixedValue(source, "band") === "S"
    && fixedValue(source, "product_type") === "RSLC";
}

/**
 * Turn the application-owned source projection into the legacy form shape
 * used by the existing filter widgets. The option list still comes entirely
 * from the projection; this function does not add a selectable source.
 */
export function datasetCapabilityForSarSource(
  source: SarSourceCapability,
): SarDatasetCapabilityProjection {
  if (source.search_kind === "legacy_asf_search") {
    const capability = projectedLegacyDatasetCapability(source);
    return capability
      ? { ok: true, capability }
      : {
          ok: false,
          error: t("shared.sarCapabilities.incompleteCapabilityProjection", { dataset: source.display_name || source.selector }),
        };
  }

  const platforms = fixedValue(source, "platform");
  const instrument = fixedValue(source, "instrument");
  const productType = fixedValue(source, "product_type") ?? fixedValue(source, "processing_level");
  const beamModes = optionValues(source, "beam_mode");
  const polarizations = fixedValue(source, "platform") === "NISAR"
    ? [...new Set([
        ...optionValues(source, "frequency_a_polarization"),
        ...optionValues(source, "frequency_b_polarization"),
        ...optionValues(source, "polarization"),
      ])]
    : optionValues(source, "polarizations");
  const flightDirections = optionValues(source, "flight_direction");
  const lookDirections = optionValues(source, "look_direction");
  const supportedActions = source.supported_actions.map((action) => ({
    action,
    platforms: [],
    product_families: [],
    required_fields: [],
  }));
  return { ok: true, capability: {
    dataset: source.selector,
    display_name: source.display_name,
    platforms: platforms ? [platforms] : [],
    instruments: instrument ? [instrument] : [],
    processing_levels: productType ? [productType] : [],
    product_families: [],
    beam_modes: beamModes,
    polarizations,
    supports_flight_direction: flightDirections.length > 0,
    supports_look_direction: lookDirections.length > 0,
    supports_relative_orbit: optionValues(source, "relative_orbit").length > 0
      || optionValues(source, "track").length > 0,
    supports_frame: optionValues(source, "frame").length > 0,
    supports_path: optionValues(source, "path").length > 0,
    supports_burst: false,
    frame_filter: null,
    track_filter: null,
    supported_actions: supportedActions,
  } };
}

function normalizedUtc(value: string, endBoundary = false): string | null {
  const trimmed = value.trim();
  const candidate = /^\d{4}-\d{2}-\d{2}$/.test(trimmed)
    ? `${trimmed}T${endBoundary ? "23:59:59" : "00:00:00"}Z`
    : trimmed;
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/.exec(candidate);
  if (!match) return null;
  const instant = new Date(candidate);
  if (!Number.isFinite(instant.getTime())) return null;
  const [year, month, day, hour, minute, second] = match.slice(1).map(Number);
  return instant.getUTCFullYear() === year
    && instant.getUTCMonth() + 1 === month
    && instant.getUTCDate() === day
    && instant.getUTCHours() === hour
    && instant.getUTCMinutes() === minute
    && instant.getUTCSeconds() === second
    ? candidate
    : null;
}

function utcBeforeOrEqual(start: string, end: string): boolean {
  const parse = (value: string): [string, string] | null => {
    const match = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?Z$/.exec(value);
    return match ? [match[1], (match[2] ?? "").padEnd(9, "0")] : null;
  };
  const left = parse(start);
  const right = parse(end);
  return Boolean(left && right && (
    left[0] < right[0] || (left[0] === right[0] && left[1] <= right[1])
  ));
}

function optionalOption(
  source: SarSourceCapability,
  key: string,
  requested: string | null | undefined,
): string | null | "invalid" {
  if (!requested) return null;
  return optionValues(source, key).includes(requested) ? requested : "invalid";
}

/** Build only the typed catalogue query owned by ALOS-2/NISAR profiles. */
export function buildTypedSarCatalogQuery(
  input: TypedSarSearchInput,
): TypedSarSearchBuildResult {
  const source = input.source;
  if (source.search_kind !== "typed_catalog_profile") {
    return { ok: false, error: t("shared.sarCapabilities.notTypedCatalogProfile") };
  }
  if (!source.available) {
    return {
      ok: false,
      error: t("shared.sarCapabilities.sourceCurrentlyUnavailable", {
        displayName: source.display_name,
        reason: source.disabled_reason ?? "route_unavailable",
      }),
    };
  }
  const aoi = input.aoiGeojson ? record(input.aoiGeojson) : null;
  if (input.aoiGeojson && !aoi) {
    return { ok: false, error: t("shared.sarCapabilities.invalidAoiGeometry") };
  }
  const rawStart = input.startUtc.trim();
  const rawEnd = input.endUtc.trim();
  const start = rawStart ? normalizedUtc(rawStart) : null;
  const end = rawEnd ? normalizedUtc(rawEnd, true) : null;
  if (rawStart && !start) {
    return { ok: false, error: t("shared.sarCapabilities.invalidStartDate") };
  }
  if (rawEnd && !end) {
    return { ok: false, error: t("shared.sarCapabilities.invalidEndDate") };
  }
  if (start && end && !utcBeforeOrEqual(start, end)) {
    return { ok: false, error: t("shared.sarCapabilities.startAfterEnd") };
  }
  if (!source.logical_dataset || !source.provider_id || !source.native_collection_id) {
    return { ok: false, error: t("shared.sarCapabilities.missingCollectionProfileId") };
  }
  if (
    input.resultLimit !== null
    && (!Number.isSafeInteger(input.resultLimit) || input.resultLimit < 1 || input.resultLimit > 10_000)
  ) {
    return { ok: false, error: t("shared.sarCapabilities.invalidTypedResultLimit") };
  }

  const filters: Record<string, unknown> = {};
  if (start || end) {
    filters.datetime = {
      ...(start ? { start } : {}),
      ...(end ? { end } : {}),
    };
  }
  if (source.selector === ALOS2_SOURCE_SELECTOR) {
    for (const [key, value] of [
      ["product_type", input.alos2ProductType],
      ["processing_level", input.alos2ProcessingLevel],
    ] as const) {
      const selected = selectedExecutableFixedChoice(source, key, value);
      if (!selected.ok) return selected;
    }
    const beamMode = optionalOption(source, "beam_mode", input.alos2BeamMode);
    if (beamMode === "invalid") {
      return { ok: false, error: t("shared.sarCapabilities.alos2InvalidBeam") };
    }
    if (beamMode) filters.beam_mode = beamMode;
    if (beamMode) {
      filters.polarizations = beamMode === "WBS" ? ["HH"] : ["HH", "HV"];
    }
    const direction = optionalOption(source, "flight_direction", input.alos2FlightDirection);
    const look = optionalOption(source, "look_direction", input.alos2LookDirection);
    if (direction === "invalid" || look === "invalid") {
      return { ok: false, error: t("shared.sarCapabilities.alos2InvalidDirectionOrLook") };
    }
    if (direction) filters.flight_direction = direction;
    if (look) filters.look_direction = look;
  } else if (isBhoonidhiSsarSource(source)) {
    for (const [key, value] of [
      ["product_type", input.nisarProductType],
      ["band", input.nisarBand],
    ] as const) {
      const selected = selectedExecutableFixedChoice(source, key, value);
      if (!selected.ok) return selected;
    }
    if (fixedValue(source, "online") !== "Y") {
      return { ok: false, error: t("shared.sarCapabilities.bhoonidhiMissingOnline") };
    }
    for (const [key, requested] of [
      ["frame_coverage", input.nisarFrameCoverage],
      ["frequency_a_polarization", input.nisarFrequencyA],
      ["frequency_b_polarization", input.nisarFrequencyB],
      ["range_bandwidth", input.nisarRangeBandwidth],
      ["flight_direction", input.nisarFlightDirection],
      ["look_direction", input.nisarLookDirection],
      ["quality_status", input.nisarMaturity],
    ] as const) {
      if (requested?.trim()) {
        return { ok: false, error: t("shared.sarCapabilities.bhoonidhiUnsupportedLBand", { key }) };
      }
    }
    for (const [key, requested] of [
      ["mode", input.bhoonidhiMode],
      ["polarization", input.bhoonidhiPolarization],
      ["acquisition_mode", input.bhoonidhiAcquisitionMode],
      ["software_version", input.bhoonidhiSoftwareVersion],
    ] as const) {
      const selected = optionalOption(source, key, requested);
      if (selected === "invalid") {
        return { ok: false, error: t("shared.sarCapabilities.bhoonidhiIllegalOption", { key }) };
      }
      if (selected) {
        filters[key] = selected;
        continue;
      }
      const choices = sarSourceChoiceValues(source, key);
      if (choices.length === 1) filters[key] = choices[0];
    }
  } else if (fixedValue(source, "platform") === "NISAR") {
    for (const [key, value] of [
      ["product_type", input.nisarProductType],
      ["band", input.nisarBand],
      ["quality_status", input.nisarMaturity],
    ] as const) {
      const selected = selectedExecutableFixedChoice(source, key, value);
      if (!selected.ok) return selected;
    }
    const fixedProduction = fixedValue(source, "production_configuration");
    if (!fixedProduction) {
      return { ok: false, error: t("shared.sarCapabilities.nisarMissingProdConfig") };
    }
    filters.production_configuration = fixedProduction;
    const choices: Array<[string, string | null | undefined]> = [
      ["frame_coverage", input.nisarFrameCoverage],
      ["frequency_a_polarization", input.nisarFrequencyA],
      ["frequency_b_polarization", input.nisarFrequencyB],
      ["range_bandwidth", input.nisarRangeBandwidth],
    ];
    for (const [key, requested] of choices) {
      const value = optionalOption(source, key, requested);
      if (value === "invalid") {
        return { ok: false, error: t("shared.sarCapabilities.nisarIllegalOption", { key }) };
      }
      if (value) filters[key] = value;
    }
    const direction = optionalOption(source, "flight_direction", input.nisarFlightDirection);
    const look = optionalOption(source, "look_direction", input.nisarLookDirection);
    if (direction === "invalid" || look === "invalid") {
      return { ok: false, error: t("shared.sarCapabilities.nisarInvalidDirectionOrLook") };
    }
    if (direction) filters.flight_direction = direction;
    if (look) filters.look_direction = look;
  } else {
    return { ok: false, error: t("shared.sarCapabilities.unknownTypedSelector") };
  }

  return {
    ok: true,
    query: {
      logical_dataset: source.logical_dataset,
      provider_id: source.provider_id,
      native_collection_id: source.native_collection_id,
      aoi_geojson: aoi,
      cursor: null,
      result_limit: input.resultLimit,
      filters,
    },
  };
}
