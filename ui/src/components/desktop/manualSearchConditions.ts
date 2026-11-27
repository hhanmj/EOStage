import type { UnifiedSearchRequest } from "@/lib/generated/v3-contracts";
import { getCurrentLocale, t } from "@/i18n";
import {
  localizeSearchFilterLabel,
  SEARCH_FILTER_SCHEMA_CATALOG,
  type SearchFilterDraft,
  type SearchFilterSchema,
} from "@/lib/searchFilterSchema";
import { sarFilterProfile } from "@/shell/api/sarFilterProfile";
import { localizedSourceDisplayName } from "@/lib/dataSourceCatalogLogic";
import { buildUnifiedSearchRequest } from "@/shell/api/searchRequest";

export type ManualConditionGroup = "source" | "aoi" | "time" | "orbit";

export type ManualConditionDraft = {
  resourceId: string;
  aoiGeometry: Record<string, unknown> | null;
  aoiName: string;
  startDate: string | null;
  endDate: string | null;
  platforms?: string[];
  productType: string;
  polarizations?: string[];
  beamModes?: string[];
  lookDirection?: string | null;
  orbitDirection: string;
  relativeOrbit: string | number | null;
  path?: string | number | null;
  frame: string | number | null;
  opticalDraft?: SearchFilterDraft;
};

export type ManualConditionTag = {
  id: string;
  label: string;
  text: string;
  group: ManualConditionGroup;
  removable: boolean;
  rawValue?: string;
};

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).sort().join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.entries(value as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

function tag(
  id: string,
  label: string,
  text: string,
  group: ManualConditionGroup,
  removable = true,
  rawValue?: string,
): ManualConditionTag {
  return { id, label, text, group, removable, rawValue };
}

function formatSatelliteToken(raw: string): string {
  const trimmed = raw.trim();
  const normalized = trimmed.toLowerCase().replace(/\s+/g, "_");
  const known: Record<string, string> = {
    landsat_9: "Landsat 9",
    landsat_8: "Landsat 8",
    landsat_7: "Landsat 7",
    landsat_5: "Landsat 5",
    landsat_4: "Landsat 4",
    sentinel_2a: "Sentinel-2A",
    sentinel_2b: "Sentinel-2B",
    sentinel_2c: "Sentinel-2C",
    "sentinel-2a": "Sentinel-2A",
    "sentinel-2b": "Sentinel-2B",
    "sentinel-2c": "Sentinel-2C",
  };
  return known[normalized] ?? trimmed;
}

function valueText(
  value: unknown,
  queryKey?: string,
  options?: ReadonlyArray<{ value: string; label?: string }>,
): string {
  const mapSingle = (raw: unknown): string => {
    const s = String(raw);
    const matched = options?.find((opt) => opt.value === s)?.label;
    if (matched) return matched;
    if (queryKey === "platforms" || queryKey === "satellites" || queryKey === "spacecraft") {
      return formatSatelliteToken(s);
    }
    return s;
  };
  if (Array.isArray(value)) {
    const items = value.map(mapSingle);
    if (
      (queryKey === "platforms" || queryKey === "satellites" || queryKey === "spacecraft") &&
      items.length > 2
    ) {
      return t("mapWorkbench.helpers.platformTruncatedCount", {
        items: items.slice(0, 2).join(t("mapWorkbench.helpers.listSeparator")),
        count: items.length,
      });
    }
    return items.join(t("mapWorkbench.helpers.listSeparator"));
  }
  return mapSingle(value);
}

function dateCondition(request: UnifiedSearchRequest): ManualConditionTag | null {
  if (!request.time_range?.start || !request.time_range.end) return null;
  const start = request.time_range.start.slice(0, 10);
  const end = request.time_range.end.slice(0, 10);
  return tag("time", t("mapWorkbench.helpers.conditionTime"), `${start} – ${end}`, "time");
}

function schemaForRequest(request: UnifiedSearchRequest): SearchFilterSchema | null {
  return SEARCH_FILTER_SCHEMA_CATALOG.schemas.find((schema) => schema.resource_id === request.resource_id) ?? null;
}

export function buildManualConditionTags(
  request: UnifiedSearchRequest,
): ManualConditionTag[] {
  const tags = [
    tag(
      "source",
      t("mapWorkbench.helpers.conditionSource"),
      localizedSourceDisplayName(request.resource_id, getCurrentLocale()),
      "source",
      false,
      request.resource_id,
    ),
  ];
  if (request.aoi) {
    tags.push(tag("aoi", t("mapWorkbench.helpers.conditionExtent"), request.aoi_name, "aoi"));
  }

  const profile = sarFilterProfile(request.resource_id);
  if (profile) {
    const time = dateCondition(request);
    if (time) tags.push(time);
    for (const [key, value] of Object.entries(request.filters)) {
      if (key === "dataset") continue;
      if (key === "processing_levels") {
        const product = profile.productTypes.find((type) => canonical(type.levels) === canonical(value));
        if (product) tags.push(tag("productType", t("mapWorkbench.helpers.conditionProductType"), product.label, "source"));
      } else if (key === "platforms") {
        tags.push(tag("platforms", t("mapWorkbench.helpers.conditionPlatforms"), valueText(value), "source"));
      } else if (key === "polarizations" || key === "polarization") {
        tags.push(tag("polarizations", t("mapWorkbench.helpers.conditionPolarization"), valueText(value), "orbit"));
      } else if (key === "beam_modes" || key === "beam_mode") {
        tags.push(tag("beamModes", t("mapWorkbench.helpers.conditionBeamMode"), valueText(value), "orbit"));
      } else if (key === "look_direction") {
        const lookText =
          value === "RIGHT"
            ? t("mapWorkbench.conditions.lookRight")
            : value === "LEFT"
              ? t("mapWorkbench.conditions.lookLeft")
              : valueText(value);
        tags.push(tag("lookDirection", t("mapWorkbench.helpers.conditionLookDirection"), lookText, "orbit"));
      } else if (key === "flight_direction") {
        tags.push(tag("orbitDirection", t("mapWorkbench.helpers.conditionOrbitDirection"), valueText(value), "orbit"));
      } else if (key === profile.relativeOrbit?.key) {
        tags.push(tag("relativeOrbit", "Path", valueText(value), "orbit"));
      } else if (key === profile.path?.key) {
        tags.push(tag("path", "Path", valueText(value), "orbit"));
      } else if (key === profile.frame?.key) {
        tags.push(tag("frame", "Frame", valueText(value), "orbit"));
      }
    }
    return tags;
  }

  const schema = schemaForRequest(request);
  const time = dateCondition(request);
  if (time) tags.push(time);
  if (!schema) return tags;
  for (const filter of schema.filters) {
    if (filter.control === "fixed" || filter.control === "date_range" || !filter.query_key) continue;
    const value = request.filters[filter.query_key];
    if (value === undefined || value === null) continue;
    if (typeof value === "string" && (value.trim() === "" || value.trim() === "留空不限制")) continue;
    const group = filter.group === "facet" ? "source" : filter.group === "time" ? "time" : "orbit";
    if (filter.query_key === "cloud_cover_lte") {
      const cloudVal = typeof value === "number" ? String(value) : String(value).trim();
      if (!cloudVal) continue;
      tags.push(
        tag(
          `optical:${filter.filter_id}`,
          localizeSearchFilterLabel(filter),
          t("mapWorkbench.helpers.maxCloudCondition", { value: cloudVal }),
          group,
        ),
      );
      continue;
    }
    tags.push(
      tag(
        `optical:${filter.filter_id}`,
        localizeSearchFilterLabel(filter),
        valueText(value, filter.query_key, (filter as { options?: Array<{ value: string; label?: string }> }).options),
        group,
      ),
    );
  }
  return tags;
}

export function manualConditionsChanged(
  frozenRequest: UnifiedSearchRequest | null,
  draft: ManualConditionDraft,
): boolean {
  if (!frozenRequest) return false;
  if (!draft.aoiGeometry || Object.keys(draft.aoiGeometry).length === 0) return true;
  try {
    const current = buildUnifiedSearchRequest({
      ...draft,
      aoiGeometry: draft.aoiGeometry,
      lookDirection:
        draft.lookDirection && draft.lookDirection !== "all"
          ? draft.lookDirection
          : null,
    });
    const { request_id: _frozenId, ...frozen } = frozenRequest;
    const { request_id: _currentId, ...request } = current;
    return canonical(frozen) !== canonical(request);
  } catch {
    return true;
  }
}
