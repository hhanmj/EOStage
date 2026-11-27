import { formatByteSize } from "@/lib/formatByteSize";
import { formatSignedLat, formatSignedLon } from "@/lib/geoCoords";
import { defaultMapFitReporter } from "@/shell/api/mapFitReporter";
import {
  initialBaseTileRuntime,
  baseTileOnTileload,
  baseTileOnTileerror,
  baseTileOnLoadEvent,
  type BaseTileRuntime,
} from "@/shell/api/mapTileStatus";
import { Fragment, memo, useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties, type ReactNode } from "react";
import type {
  LatLng,
  LatLngBoundsExpression,
  LatLngExpression,
  LeafletMouseEvent,
  Map as LeafletMap,
} from "leaflet";
import { DomEvent } from "leaflet";
import {
  AttributionControl,
  MapContainer,
  Pane,
  CircleMarker,
  ImageOverlay,
  Polyline,
  Polygon,
  Rectangle,
  TileLayer,
  useMap,
  useMapEvents,
} from "@/components/map/leafletBindings";
import {
  ArrowUpRight,
  Check,
  ChevronDown,
  Eye,
  EyeOff,
  Info,
  Layers,
  ListTree,
  LocateFixed,
  MapPinned,
  Minus,
  Pentagon,
  Plus,
  Square,
  SquareDashed,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import {
  importVectorFromHtml5Files,
  importVectorFromNativePaths,
  useNativeVectorDropTarget,
} from "@/lib/vectorDropImport";
import type { Bbox, Json, SceneRow } from "@/lib/bridge";
import {
  ASF_SENTINEL1_GRD_GEOGRAPHIC_PREVIEW_PROFILE,
  type PreviewRuntimeState,
} from "@/lib/catalogPreview";
import {
  MAP_PREVIEW_PANE_Z_INDEX,
  MAP_TILE_Z_INDEX,
  effectiveAnnotationOverlays,
  mapSummaryBbox,
  type AnnotationOverlayPreference,
} from "@/lib/mapLayerOrder";
import {
  DEFAULT_MAP_PROVIDER,
  MAP_PROVIDERS,
  isEnabledMapProvider,
  type MapProviderKey,
} from "@/lib/mapProviders";
import { getGeeLayerStoreSnapshot, subscribeGeeLayerStore } from "@/lib/geeCloudSuiteClient";
import { MAP_ANNOTATION_PROVIDERS, type MapAnnotationProviderKey } from "@/lib/mapAnnotationProviders";
import {
  effectiveTiandituToken,
  hasEffectiveTiandituToken,
} from "@/lib/mapRuntimeCredentials";
import {
  projectBboxForMapProvider,
  projectGeometryForMapProvider,
} from "@/lib/mapProviderGeometry";
import { sceneProductKey } from "@/lib/sarProduct";
import { formatOrbitDirection, sceneListLabel, extractCloudCover } from "@/shell/api/rowProjection";
import { describeSceneStack } from "@/shell/api/mapLinkLogic";
import { sceneFootprintRenderModel, type SceneFootprintRenderModel } from "@/lib/sceneFootprint";
import type { CoreAoiDisplayAuthorityProjection } from "@/lib/coreRestoreProjection";
import {
  canonicalLeafletLongitude,
  canonicalizeLeafletPolygonRing,
  canonicalizeLeafletRectangle,
  continuousDisplayBbox,
  deriveWrappedAoiEnvelope,
  expandedDemDisplayBbox,
  shiftBboxToDisplayDomain,
  shiftPolygonalGeojsonToDisplayDomain,
  shiftPolygonalGeojsonToLongitudeReference,
} from "@/lib/antimeridian";
import { DEM_BBOX_BUFFER_DEGREES } from "@/lib/demSources";
import type { LandsatMapTilePreview } from "@/lib/landsatCatalog";
import type { Sentinel2MapTilePreview } from "@/lib/sentinel2Catalog";
import { constrainMapTooltipPosition } from "@/lib/mapTooltipPosition";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import { geometryAreaKm2 } from "@/lib/geometryArea";
import { GOLDEN_MAP_SCENE_LIMIT } from "@/lib/workbenchMapProjection";
import { isUiSmokeMode, markUiSmoke } from "@/lib/uiSmoke";
import { cn } from "@/lib/utils";
import { t as translateNow, useI18n } from "@/i18n";
import { SceneFootprintLayer } from "./SceneFootprintLayer";
import { AoiOverlay } from "./AoiOverlay";
import { applyPreviewPolygonClip, PreviewImageOverlay } from "./PreviewImageOverlay";
import type { PinnedMapPreviewEntry } from "@/hooks/useWorkbenchMapPreview";
import {
  GeeLayerStackHud,
  GeeMapLeafletOverlays,
  type MapSwipeLayerSpec,
} from "./desktop/GeeLayerStackAndSwipe";

export type { MapSwipeLayerSpec };

export type WorkbenchDrawMode = "rect" | "polygon" | "point";
export type MapLayerKey = MapProviderKey;

export interface GeodataRasterOverlaySpec {
  id: string;
  label?: string;
  previewDataUrl?: string | null;
  tileUrlTemplate?: string | null;
  bbox?: Bbox | null;
  attribution?: string;
  maxZoom?: number;
}

export const MAP_CHIP_BASE_CLASS =
  "h-7 rounded-sp-sm border border-sp-line bg-sp-surface/95 px-2.5 text-xs text-sp-text-secondary shadow-overlay";
export const MAP_CHIP_ICON_CLASS =
  "h-7 w-7 rounded-sp-sm border border-sp-line bg-sp-surface/95 text-xs text-sp-text-secondary shadow-overlay flex items-center justify-center";

export type MapRuntimeStatus =
  | "map_initializing"
  | "external_tiles_loading"
  | "external_tiles_ready"
  | "external_tiles_failed"
  | "csp_or_network_blocked";

const DEFAULT_BBOX: Bbox = {
  west: 73.5,
  east: 135.1,
  south: 18.0,
  north: 53.6,
  crs: "EPSG:4326",
};
const UI_SMOKE_AOI_BBOX: Bbox = {
  west: 116.1,
  east: 116.5,
  south: 39.7,
  north: 40.0,
  crs: "EPSG:4326",
};
const MAP_SCENE_DISPLAY_LIMIT = Math.max(250, GOLDEN_MAP_SCENE_LIMIT);

export type PreviewRenderAuthority = {
  region_id: string;
  revision: number;
  render_geometry: Json | null;
  render_bbox: Bbox | null;
};

type MapLayerConfig = {
  label: string;
  desc: string;
  url: string;
  subdomains: string[];
  attribution: string;
  attributionFull?: string;
  maxZoom?: number;
  requiresToken: boolean;
  category: string;
  containsLabels: boolean;
  enabledInPreview: boolean;
  crs: string;
};

const BASEMAP_LABEL_KEYS: Record<MapProviderKey, Parameters<typeof translateNow>[0]> = {
  osm: "panels.map.basemapLabelOsm",
  cartoLight: "panels.map.basemapLabelCartoLight",
  cartoDark: "panels.map.basemapLabelCartoDark",
  arcgisSatellite: "panels.map.basemapLabelArcgisSatellite",
  arcgisTopo: "panels.map.basemapLabelArcgisTopo",
  arcgisStreet: "panels.map.basemapLabelArcgisStreet",
  openTopoMap: "panels.map.basemapLabelOpenTopoMap",
  googleSatellite: "panels.map.basemapLabelGoogleSatellite",
  googleMap: "panels.map.basemapLabelGoogleMap",
  googleHybrid: "panels.map.basemapLabelGoogleHybrid",
  gaodeMap: "panels.map.basemapLabelGaodeMap",
  gaodeSatellite: "panels.map.basemapLabelGaodeSatellite",
  tiandituSatellite: "panels.map.basemapLabelTiandituSatellite",
  tiandituVector: "panels.map.basemapLabelTiandituVector",
};

const ANNOTATION_LABEL_KEYS: Record<MapAnnotationProviderKey, Parameters<typeof translateNow>[0]> = {
  tiandituImageLabels: "panels.map.annotationLabelTiandituImage",
  tiandituVectorLabels: "panels.map.annotationLabelTiandituVector",
};

const BASEMAP_CATEGORY_NOTE_KEYS: Partial<Record<MapProviderKey, Parameters<typeof translateNow>[0]>> = {
  googleSatellite: "panels.map.categoryNoteProxyRequired",
  googleMap: "panels.map.categoryNoteProxyRequired",
  googleHybrid: "panels.map.categoryNoteProxyRequired",
  gaodeMap: "panels.map.categoryNoteGcj02Reference",
  gaodeSatellite: "panels.map.categoryNoteGcj02Reference",
};

const BASEMAP_SHORT_DESC_KEYS: Record<MapProviderKey, Parameters<typeof translateNow>[0]> = {
  osm: "panels.map.basemapDescOsm",
  cartoLight: "panels.map.basemapDescCartoLight",
  cartoDark: "panels.map.basemapDescCartoDark",
  arcgisSatellite: "panels.map.basemapDescArcgisSatellite",
  arcgisTopo: "panels.map.basemapDescArcgisTopo",
  arcgisStreet: "panels.map.basemapDescArcgisStreet",
  openTopoMap: "panels.map.basemapDescOpenTopoMap",
  googleSatellite: "panels.map.basemapDescGoogleSatellite",
  googleMap: "panels.map.basemapDescGoogleMap",
  googleHybrid: "panels.map.basemapDescGoogleHybrid",
  gaodeMap: "panels.map.basemapDescGaodeMap",
  gaodeSatellite: "panels.map.basemapDescGaodeSatellite",
  tiandituSatellite: "panels.map.basemapDescTiandituSatellite",
  tiandituVector: "panels.map.basemapDescTiandituVector",
};

const MAP_LAYERS = Object.fromEntries(
  MAP_PROVIDERS.map((provider) => [
    provider.key,
    {
      get label() {
        const key = BASEMAP_LABEL_KEYS[provider.key];
        return key ? translateNow(key) : provider.display_name;
      },
      get desc() {
        const key = BASEMAP_SHORT_DESC_KEYS[provider.key];
        return key ? translateNow(key) : provider.display_name;
      },
      url: provider.tile_url_template,
      subdomains: provider.subdomains,
      attribution: provider.attribution,
      attributionFull: provider.attribution_full,
      maxZoom: provider.key === "openTopoMap" ? 17 : 19,
      requiresToken: provider.requires_token,
      get category() {
        const noteKey = BASEMAP_CATEGORY_NOTE_KEYS[provider.key];
        if (noteKey) return translateNow(noteKey);
        return provider.category_note
          ? provider.category_note
          : translateNow("panels.map.reviewedPreviewSource");
      },
      containsLabels: provider.contains_labels,
      enabledInPreview: provider.enabled_in_preview,
      crs: provider.crs ?? "EPSG:3857",
    } satisfies MapLayerConfig,
  ]),
) as Record<MapLayerKey, MapLayerConfig>;

const DEFAULT_ANNOTATION_PREFERENCE: AnnotationOverlayPreference = {
  image: true,
  vector: false,
};

function tileUrlWithUserToken(template: string, token: string): string {
  return template.replaceAll("{token}", encodeURIComponent(token.trim()));
}

function fmt(v: number): string {
  return Number.isFinite(v) ? v.toFixed(4) : "-";
}

function sceneIdentity(scene: SceneRow): string {
  return sceneProductKey(scene);
}

function boundsFromBbox(bbox: Bbox): LatLngBoundsExpression {
  return [
    [bbox.south, bbox.west],
    [bbox.north, bbox.east],
  ];
}

function centerFromBbox(bbox: Bbox): LatLngExpression {
  return [(bbox.south + bbox.north) / 2, (bbox.west + bbox.east) / 2];
}

function isValidBbox(bbox: Bbox | null | undefined): bbox is Bbox {
  return Boolean(
    bbox
    && [bbox.west, bbox.east, bbox.south, bbox.north].every(Number.isFinite)
    && bbox.west < bbox.east
    && bbox.south < bbox.north,
  );
}

function parseTupleOrObjectBbox(raw: unknown): Bbox | null {
  if (Array.isArray(raw) && raw.length >= 4) {
    const west = Number(raw[0]);
    const south = Number(raw[1]);
    const east = Number(raw[2]);
    const north = Number(raw[3]);
    const cand = { west, south, east, north };
    return isValidBbox(cand) ? cand : null;
  }
  if (raw && typeof raw === "object") {
    const obj = raw as Record<string, unknown>;
    const cand = {
      west: Number(obj.west),
      south: Number(obj.south),
      east: Number(obj.east),
      north: Number(obj.north),
    };
    return isValidBbox(cand) ? cand : null;
  }
  return null;
}

export function extractGeodataRasterOverlayFromResponse(
  res: any,
  fallbackBbox: Bbox | null | undefined,
  label?: string,
): GeodataRasterOverlaySpec | null {
  if (!res || typeof res !== "object") return null;
  const rawItems = Array.isArray(res.items) ? res.items : [];
  const firstItemWithRaster =
    rawItems.find(
      (it: any) =>
        it &&
        (typeof it.preview_data_url === "string" ||
          typeof it.tile_url_template === "string" ||
          typeof it.url_format === "string"),
    ) ?? rawItems[0];

  const rawPreviewUrl =
    res.preview_data_url ||
    res.mosaic_preview_data_url ||
    firstItemWithRaster?.preview_data_url ||
    null;
  const previewDataUrl =
    typeof rawPreviewUrl === "string" && rawPreviewUrl.trim().length > 0
      ? rawPreviewUrl.trim()
      : null;

  const rawTileTemplate =
    res.tile_url_template ||
    res.url_format ||
    firstItemWithRaster?.tile_url_template ||
    firstItemWithRaster?.url_format ||
    (typeof res.custom_url_template === "string" && res.custom_url_template.includes("{z}")
      ? res.custom_url_template
      : null) ||
    null;
  const tileUrlTemplate =
    typeof rawTileTemplate === "string" &&
    rawTileTemplate.trim().length > 0 &&
    (rawTileTemplate.includes("{z}") ||
      rawTileTemplate.startsWith("http") ||
      rawTileTemplate.startsWith("data:"))
      ? rawTileTemplate.trim()
      : null;

  if (!previewDataUrl && !tileUrlTemplate) return null;

  const resolvedBbox =
    parseTupleOrObjectBbox(res.mosaic_bbox) ||
    parseTupleOrObjectBbox(res.bbox) ||
    parseTupleOrObjectBbox(res.requested_bbox_wgs84) ||
    parseTupleOrObjectBbox(firstItemWithRaster?.mosaic_bbox) ||
    parseTupleOrObjectBbox(firstItemWithRaster?.bbox) ||
    (isValidBbox(fallbackBbox) ? fallbackBbox : null);

  if (!tileUrlTemplate && !resolvedBbox) return null;

  const idSeed = String(
    res.exported_mosaic_path ||
      res.mosaic_geotiff_path ||
      res.mbtiles_path ||
      res.zip_archive_path ||
      res.downloaded_file ||
      res.downloaded_path ||
      res.asset_id ||
      res.dataset ||
      label ||
      "geodata-raster",
  );

  return {
    id: `${idSeed}:${previewDataUrl ? previewDataUrl.slice(0, 48) : ""}:${tileUrlTemplate || ""}`,
    label: label || String(res.title || res.dataset || translateNow("panels.map.rasterOverlayFallback")),
    previewDataUrl,
    tileUrlTemplate,
    bbox: resolvedBbox,
    attribution: typeof res.attribution === "string" ? res.attribution : undefined,
    maxZoom:
      typeof res.max_zoom === "number"
        ? res.max_zoom
        : typeof res.zoom === "number"
          ? Math.max(19, res.zoom)
          : 20,
  };
}

function isLocalDisplayBbox(bbox: Bbox | null | undefined): bbox is Bbox {
  return isValidBbox(bbox) && bbox.east - bbox.west <= 180;
}

function candidateViewBbox(candidates: Bbox[]): Bbox | null {
  const local = candidates.filter(isLocalDisplayBbox);
  if (local.length === 0) return null;
  if (local.length === 1) return local[0]!;
  const merged = unionBbox(local);
  if (isLocalDisplayBbox(merged) && merged.east - merged.west <= 60 && merged.north - merged.south <= 45) {
    return merged;
  }
  return local[0]!;
}

/**
 * Render-only footprint fallback for legacy/catalog rows that carry only a
 * canonical bbox. This is deliberately kept local to the map: no geometry is
 * persisted and an administrative bbox is never promoted to an AOI geometry.
 */
function sceneFootprintFromBbox(
  bbox: Bbox | null | undefined,
): ReturnType<typeof sceneFootprintRenderModel> {
  if (!isValidBbox(bbox)) return null;
  const geojson: Json = {
    type: "Polygon",
    coordinates: [[
      [bbox.west, bbox.south],
      [bbox.east, bbox.south],
      [bbox.east, bbox.north],
      [bbox.west, bbox.north],
      [bbox.west, bbox.south],
    ]],
  };
  return sceneFootprintRenderModel(geojson);
}

function isFullWorldDisplayEnvelope(envelope: ReturnType<typeof deriveWrappedAoiEnvelope>): boolean {
  return Boolean(
    envelope &&
    !envelope.crossesAntimeridian &&
    envelope.displayWest === -180 &&
    envelope.displayEast === 180,
  );
}

function polarizationLabel(value: string | null | undefined, missing = "-"): string {
  const upper = (value || "").toUpperCase();
  const labels: Record<string, string> = {
    DV: "VV+VH",
    DH: "HH+HV",
    SV: "VV",
    SH: "HH",
  };
  return labels[upper] ? `${labels[upper]} (${upper})` : upper || missing;
}

export const MAP_ASC_COLOR = "#2563eb";
export const MAP_DESC_COLOR = "#f97316";

function orbitDirectionColor(value: string | null | undefined): string {
  const upper = (value || "").toUpperCase();
  if (upper === "ASCENDING" || upper === "A") return MAP_ASC_COLOR;
  if (upper === "DESCENDING" || upper === "D") return MAP_DESC_COLOR;
  return "#64748b";
}

function unionBbox(boxes: (Bbox | null | undefined)[]): Bbox | null {
  const valid = boxes.filter(isValidBbox);
  if (!valid.length) return null;
  let west = valid[0]!.west;
  let east = valid[0]!.east;
  let south = valid[0]!.south;
  let north = valid[0]!.north;
  for (let index = 1; index < valid.length; index += 1) {
    const box = valid[index]!;
    west = Math.min(west, box.west);
    east = Math.max(east, box.east);
    south = Math.min(south, box.south);
    north = Math.max(north, box.north);
  }
  return {
    west,
    east,
    south,
    north,
    crs: "EPSG:4326",
  };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function ringToLatLngs(value: unknown): LatLngExpression[] | null {
  if (!Array.isArray(value)) return null;
  const ring: LatLngExpression[] = [];
  for (const item of value) {
    if (
      Array.isArray(item) &&
      item.length >= 2 &&
      typeof item[0] === "number" &&
      typeof item[1] === "number"
    ) {
      ring.push([item[1], item[0]]);
    }
  }
  return ring.length >= 3 ? ring : null;
}

function polygonFromCoordinates(value: unknown): LatLngExpression[][] | null {
  if (!Array.isArray(value)) return null;
  const rings = value.map(ringToLatLngs).filter((ring): ring is LatLngExpression[] => !!ring);
  return rings.length ? rings : null;
}

function geometryPolygons(value: unknown): LatLngExpression[][][] {
  const geometry = asRecord(value);
  if (!geometry) return [];
  const type = String(geometry.type || "");
  const coordinates = geometry.coordinates;
  if (type === "Polygon") {
    const polygon = polygonFromCoordinates(coordinates);
    return polygon ? [polygon] : [];
  }
  if (type === "MultiPolygon" && Array.isArray(coordinates)) {
    return coordinates
      .map(polygonFromCoordinates)
      .filter((polygon): polygon is LatLngExpression[][] => !!polygon);
  }
  if (type === "Feature") {
    return geometryPolygons(geometry.geometry);
  }
  if (type === "FeatureCollection" && Array.isArray(geometry.features)) {
    return geometry.features.flatMap((feature) => geometryPolygons(feature));
  }
  return [];
}

function projectSceneFootprintForMapProvider(
  footprint: NonNullable<ReturnType<typeof sceneFootprintRenderModel>>,
  providerCrs: string,
) {
  if (providerCrs !== "GCJ-02") return footprint;
  const displayGeometry: Json = {
    type: "MultiPolygon",
    coordinates: footprint.polygons.map((polygon) => polygon.map((ring) =>
      ring.map(([latitude, longitude]) => [longitude, latitude]))),
  };
  const projectedGeometry = projectGeometryForMapProvider(displayGeometry, providerCrs);
  return {
    ...footprint,
    geojson: projectedGeometry,
    bbox: projectBboxForMapProvider(footprint.bbox, providerCrs),
    polygons: geometryPolygons(projectedGeometry) as [number, number][][][],
  };
}

function xmlEscape(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function fmtCoord(value: number): string {
  return Number(value.toFixed(7)).toString();
}

function kmlCoordinateText(ring: unknown): string | null {
  if (!Array.isArray(ring)) return null;
  const points: [number, number][] = [];
  for (const item of ring) {
    if (
      Array.isArray(item) &&
      item.length >= 2 &&
      typeof item[0] === "number" &&
      typeof item[1] === "number" &&
      Number.isFinite(item[0]) &&
      Number.isFinite(item[1])
    ) {
      points.push([item[0], item[1]]);
    }
  }
  if (points.length < 3) return null;
  const first = points[0];
  const last = points[points.length - 1];
  const closed = first[0] === last[0] && first[1] === last[1] ? points : [...points, first];
  return closed.map(([lng, lat]) => `${fmtCoord(lng)},${fmtCoord(lat)},0`).join(" ");
}

function polygonCoordinatesToKml(coordinates: unknown): string | null {
  if (!Array.isArray(coordinates) || coordinates.length === 0) return null;
  const outer = kmlCoordinateText(coordinates[0]);
  if (!outer) return null;
  const inner = coordinates
    .slice(1)
    .map(kmlCoordinateText)
    .filter((value): value is string => !!value)
    .map((value) => `<innerBoundaryIs><LinearRing><coordinates>${value}</coordinates></LinearRing></innerBoundaryIs>`)
    .join("");
  return `<Polygon><outerBoundaryIs><LinearRing><coordinates>${outer}</coordinates></LinearRing></outerBoundaryIs>${inner}</Polygon>`;
}

function geometryToKmlPolygons(value: unknown): string[] {
  const geometry = asRecord(value);
  if (!geometry) return [];
  const type = String(geometry.type || "");
  const coordinates = geometry.coordinates;
  if (type === "Polygon") {
    const polygon = polygonCoordinatesToKml(coordinates);
    return polygon ? [polygon] : [];
  }
  if (type === "MultiPolygon" && Array.isArray(coordinates)) {
    return coordinates
      .map(polygonCoordinatesToKml)
      .filter((polygon): polygon is string => !!polygon);
  }
  if (type === "Feature") return geometryToKmlPolygons(geometry.geometry);
  if (type === "FeatureCollection" && Array.isArray(geometry.features)) {
    return geometry.features.flatMap((feature) => geometryToKmlPolygons(feature));
  }
  if (type === "GeometryCollection" && Array.isArray(geometry.geometries)) {
    return geometry.geometries.flatMap((item) => geometryToKmlPolygons(item));
  }
  return [];
}

function countGeometryVertices(value: unknown): number {
  const geometry = asRecord(value);
  if (!geometry) return 0;
  const type = String(geometry.type || "");
  const coordinates = geometry.coordinates;
  if (type === "Polygon" && Array.isArray(coordinates)) {
    return coordinates.reduce((sum, ring) => {
      if (!Array.isArray(ring)) return sum;
      const closed =
        ring.length > 1 &&
        Array.isArray(ring[0]) &&
        Array.isArray(ring[ring.length - 1]) &&
        ring[0][0] === ring[ring.length - 1][0] &&
        ring[0][1] === ring[ring.length - 1][1];
      return sum + Math.max(0, ring.length - (closed ? 1 : 0));
    }, 0);
  }
  if (type === "MultiPolygon" && Array.isArray(coordinates)) {
    return coordinates.reduce((sum, polygon) => sum + countGeometryVertices({ type: "Polygon", coordinates: polygon }), 0);
  }
  if (type === "Feature") return countGeometryVertices(geometry.geometry);
  if (type === "FeatureCollection" && Array.isArray(geometry.features)) {
    return geometry.features.reduce((sum, feature) => sum + countGeometryVertices(feature), 0);
  }
  if (type === "GeometryCollection" && Array.isArray(geometry.geometries)) {
    return geometry.geometries.reduce((sum, item) => sum + countGeometryVertices(item), 0);
  }
  return 0;
}

function bboxPolygonKml(bbox: Bbox): string {
  const ring = [
    [bbox.west, bbox.south],
    [bbox.east, bbox.south],
    [bbox.east, bbox.north],
    [bbox.west, bbox.north],
    [bbox.west, bbox.south],
  ];
  return polygonCoordinatesToKml([ring]) ?? "";
}

function kmlDocument(name: string, bbox: Bbox, polygons: string[]): string {
  const geometry =
    polygons.length === 1
      ? polygons[0]
      : `<MultiGeometry>${polygons.join("")}</MultiGeometry>`;
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<kml xmlns="http://www.opengis.net/kml/2.2">',
    "<Document>",
    `<name>${xmlEscape(name)}</name>`,
    "<Placemark>",
    `<name>${xmlEscape(name)}</name>`,
    "<ExtendedData>",
    `<Data name="west"><value>${fmtCoord(bbox.west)}</value></Data>`,
    `<Data name="south"><value>${fmtCoord(bbox.south)}</value></Data>`,
    `<Data name="east"><value>${fmtCoord(bbox.east)}</value></Data>`,
    `<Data name="north"><value>${fmtCoord(bbox.north)}</value></Data>`,
    '<Data name="crs"><value>EPSG:4326</value></Data>',
    "</ExtendedData>",
    geometry,
    "</Placemark>",
    "</Document>",
    "</kml>",
  ].join("\n");
}

function isOpticalScene(scene: SceneRow): boolean {
  const dataset = String(scene.dataset ?? "").trim().toLowerCase();
  const provider = String(scene.provider ?? "").trim().toLowerCase();
  const productType = String(scene.product_type ?? "").trim().toUpperCase();
  const family = String(scene.product_family ?? "").trim().toLowerCase();
  const platform = String(scene.platform ?? "").trim().toLowerCase();
  const nativeLogical = String(
    (scene.native_metadata as Record<string, unknown> | undefined)?.logical_dataset ?? "",
  ).trim().toLowerCase();
  return dataset.startsWith("optical.")
    || nativeLogical.startsWith("optical.")
    || family.startsWith("optical")
    || /(^|[^a-z])(hls|landsat|sentinel-2|modis)([^a-z]|$)/.test(dataset)
    || /^(landsat|sentinel-2|hls)/.test(platform)
    || platform === "terra" || platform === "aqua"
    || provider === "copernicus.dataspace.odata"
    || provider === "element84.earth-search"
    || provider === "usgs.eros.m2m"
    || (provider === "microsoft.planetary-computer"
      && (dataset.includes("landsat") || dataset.includes("sentinel-2") || productType.startsWith("S2MSI")));
}

function sceneDownloadEntryLabel(scene: SceneRow): string {
  if (scene.has_url && scene.download_url?.trim()) return translateNow("panels.map.downloadEntryFrozen");
  const provider = String(scene.provider ?? "").trim().toLowerCase();
  const dataset = String(scene.dataset ?? "").trim().toLowerCase();
  const productType = String(scene.product_type ?? "").trim().toUpperCase();
  const microsoftSentinel2 = provider === "microsoft.planetary-computer"
    && (dataset.includes("sentinel-2") || productType.startsWith("S2MSI"));
  if (provider === "element84.earth-search") return translateNow("panels.map.downloadEntrySearchOnly");
  if (microsoftSentinel2 || isOpticalScene(scene)) return translateNow("panels.map.downloadEntryOnDemand");
  return scene.has_url ? translateNow("panels.map.downloadEntryProvided") : translateNow("panels.map.downloadEntryNotProvided");
}

function truncateMiddle(str: string, maxLen = 24): string {
  if (str.length <= maxLen) return str;
  const front = Math.ceil((maxLen - 1) / 2);
  const back = Math.floor((maxLen - 1) / 2);
  return `${str.slice(0, front)}…${str.slice(-back)}`;
}

export type ScenePopupTitleMode = "scene_id" | "display_name";

export function ScenePopup({
  scene,
  index,
  heading = translateNow("panels.map.orbitHeading"),
  stackSize,
  titleMode = "scene_id",
  hideDownloadEntry = false,
}: {
  scene: SceneRow;
  index: number;
  heading?: string;
  stackSize?: number;
  /** display_name：标题用文件名（为空时「未命名」），不显示裸 provider ID */
  titleMode?: ScenePopupTitleMode;
  /** 新外壳的行投影不携带下载地址，「下载入口」一行对它没有意义 */
  hideDownloadEntry?: boolean;
}) {
  const { t } = useI18n();
  const shellPresentation = titleMode === "display_name";
  const title = shellPresentation ? sceneListLabel(scene) : scene.scene_id;
  const productParts = [scene.product_type, scene.beam_mode, scene.polarization];
  const showProductLine =
    !shellPresentation || productParts.some((part) => typeof part === "string" && part.trim().length > 0);
  // 新外壳缺失值写「未知」；旧界面保持「-」
  const missing = shellPresentation ? t("panels.map.unknown") : "-";
  return (
    <div className={cn("min-w-[230px] max-w-[300px] text-xs", shellPresentation && "pb-1")}>
      <div className="mb-2 flex items-start gap-2">
        <span className="mt-0.5 rounded-full bg-primary/10 px-2 py-0.5 font-mono text-[11px] text-primary">
          #{index + 1}
        </span>
        <div className="min-w-0">
          <div className="break-all font-mono text-[11px] font-semibold leading-4">
            {title}
          </div>
          {showProductLine && (
            <div className="mt-1 text-muted-foreground">
              {scene.product_type || missing} · {scene.beam_mode || missing} · {polarizationLabel(scene.polarization, missing)}
            </div>
          )}
          {stackSize !== undefined && stackSize >= 2 && (
            <div className="mt-1 font-medium text-sp-accent-strong">
              {t("panels.map.sameTrackStack", { scenes: stackSize })}
            </div>
          )}
        </div>
      </div>
      <div className="grid grid-cols-2 gap-x-3 gap-y-1">
        {scene.orbit_direction?.trim() && (
          <>
            <span className="text-muted-foreground">{heading}</span>
            <span className="text-right">
              {shellPresentation ? formatOrbitDirection(scene.orbit_direction) : scene.orbit_direction}
            </span>
          </>
        )}
        {(scene.path !== null && scene.path !== undefined) || (scene.relative_orbit !== null && scene.relative_orbit !== undefined) ? (
          <>
            <span className="text-muted-foreground">Path</span>
            <span className="text-right">{scene.path ?? scene.relative_orbit}</span>
          </>
        ) : null}
        {scene.frame !== null && scene.frame !== undefined && (
          <>
            <span className="text-muted-foreground">Frame</span>
            <span className="text-right">{scene.frame}</span>
          </>
        )}
        {scene.absolute_orbit !== null && scene.absolute_orbit !== undefined && (
          <>
            <span className="text-muted-foreground">{t("panels.map.absoluteOrbit")}</span>
            <span className="text-right">{scene.absolute_orbit}</span>
          </>
        )}
        <span className="text-muted-foreground">{t("panels.map.time")}</span>
        <span className="text-right">{scene.acquisition_datetime || missing}</span>
        <span className="text-muted-foreground">{t("panels.map.size")}</span>
        <span className="text-right">{formatByteSize(scene.file_size_remote, missing)}</span>
        {!hideDownloadEntry && (
          <>
            <span className="text-muted-foreground">{t("panels.map.downloadEntry")}</span>
            <span
              className="text-right"
              title={t("panels.map.downloadEntryHint")}
            >
              {sceneDownloadEntryLabel(scene)}
            </span>
          </>
        )}
        {scene.footprint_bbox && (
          <>
            <span className="text-muted-foreground">{t("panels.map.footprintBox")}</span>
            <span className="text-right font-mono">
              {formatSignedLon(scene.footprint_bbox.west)} {formatSignedLat(scene.footprint_bbox.south)}
              <br />
              {formatSignedLon(scene.footprint_bbox.east)} {formatSignedLat(scene.footprint_bbox.north)}
            </span>
          </>
        )}
      </div>
    </div>
  );
}

type SceneTooltipState = {
  scene: SceneRow;
  index: number;
  anchor: { x: number; y: number };
};

type AoiTooltipState = {
  anchor: { x: number; y: number };
};

const MapSceneFootprintEntry = memo(function MapSceneFootprintEntry({
  scene,
  footprint,
  index,
  selected,
  hovered = false,
  drawActive,
  onSceneSelect,
  onTooltipShow,
  onTooltipHide,
}: {
  scene: SceneRow;
  footprint: SceneFootprintRenderModel;
  index: number;
  selected: boolean;
  hovered?: boolean;
  drawActive: boolean;
  onSceneSelect: (sceneId: string) => void;
  onTooltipShow: (
    scene: SceneRow,
    index: number,
    footprint: SceneFootprintRenderModel,
    event: LeafletMouseEvent,
  ) => void;
  onTooltipHide: () => void;
}) {
  const footprintColor = isOpticalScene(scene)
    ? "#8b5cf6"
    : orbitDirectionColor(scene.orbit_direction);
  return (
    <SceneFootprintLayer
      scene={scene}
      footprint={footprint}
      selected={selected}
      drawActive={drawActive}
      onSceneSelect={onSceneSelect}
      renderPart={({ key, positions, className, selected: partSelected, onClick }) => {
        const isHovered = Boolean(hovered && !partSelected);
        const isSelectedAndHovered = Boolean(hovered && partSelected);
        const options = {
          color: footprintColor,
          weight: partSelected ? 4.8 : isHovered ? 3.2 : 1.8,
          opacity: partSelected ? 1 : isHovered ? 0.95 : 0.82,
          fillColor: footprintColor,
          fillOpacity: 0,
          className: cn(className, isHovered && "scene-footprint--hovered"),
        };
        const haloOptions = {
          color: "#ffffff",
          weight: 9,
          opacity: 0.92,
          fillOpacity: 0,
          className: "scene-footprint scene-footprint--selected",
        };
        const hoverHaloOptions = {
          color: "#fde047",
          weight: 7,
          opacity: 0.9,
          fillOpacity: 0,
          className: "scene-footprint scene-footprint--hover-halo",
        };
        const handleSceneClick = (event: LeafletMouseEvent) => {
          event.originalEvent.preventDefault();
          event.originalEvent.stopPropagation();
          onClick();
        };
        const showSceneTooltip = (event: LeafletMouseEvent) => {
          onTooltipShow(scene, index, footprint, event);
        };
        return (
          <Fragment key={key}>
            {partSelected && <Polygon positions={positions} pathOptions={haloOptions} interactive={false} />}
            {isHovered && <Polygon positions={positions} pathOptions={hoverHaloOptions} interactive={false} />}
            <Polygon
              positions={positions}
              pathOptions={options}
              interactive={!drawActive}
              eventHandlers={{
                click: handleSceneClick,
                mouseover: showSceneTooltip,
                mousemove: showSceneTooltip,
                mouseout: onTooltipHide,
              }}
            />
          </Fragment>
        );
      }}
    />
  );
});

function BoundedSceneTooltip({
  map,
  state,
  heading,
  stackSize,
  popupTitleMode,
  hideDownloadEntry,
}: {
  map: LeafletMap | null;
  state: SceneTooltipState | null;
  heading?: string;
  stackSize?: number;
  popupTitleMode?: ScenePopupTitleMode;
  hideDownloadEntry?: boolean;
}) {
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [tooltipSize, setTooltipSize] = useState({ width: 300, height: 220 });

  useLayoutEffect(() => {
    if (!map) return;
    const container = map.getContainer();
    const updateViewport = () => setViewport({
      width: container.clientWidth,
      height: container.clientHeight,
    });
    updateViewport();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateViewport);
    observer.observe(container);
    return () => observer.disconnect();
  }, [map]);

  useLayoutEffect(() => {
    const element = tooltipRef.current;
    if (!element || !state) return;
    const rect = element.getBoundingClientRect();
    setTooltipSize({ width: rect.width, height: rect.height });
  }, [state]);

  if (!state || viewport.width <= 0 || viewport.height <= 0) return null;
  const position = constrainMapTooltipPosition(state.anchor, viewport, tooltipSize, 10);
  return (
    <div
      ref={tooltipRef}
      role="tooltip"
      className="pointer-events-none absolute z-[700] max-h-[calc(100%-20px)] max-w-[calc(100%-20px)] overflow-auto rounded-lg border border-slate-200 bg-white px-3 py-2 text-slate-900 shadow-xl dark:border-slate-700 dark:bg-slate-950 dark:text-slate-100"
      style={{ left: position.x, top: position.y }}
    >
      <ScenePopup scene={state.scene} index={state.index} heading={heading} stackSize={stackSize}
        titleMode={popupTitleMode} hideDownloadEntry={hideDownloadEntry} />
    </div>
  );
}

function BoundedAoiTooltip({
  map,
  state,
  name,
  kind,
  areaKm2,
}: {
  map: LeafletMap | null;
  state: AoiTooltipState | null;
  name: string;
  kind: string;
  areaKm2: number;
}) {
  const { t } = useI18n();
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [tooltipSize, setTooltipSize] = useState({ width: 220, height: 100 });

  useLayoutEffect(() => {
    if (!map) return;
    const container = map.getContainer();
    const updateViewport = () => setViewport({
      width: container.clientWidth,
      height: container.clientHeight,
    });
    updateViewport();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateViewport);
    observer.observe(container);
    return () => observer.disconnect();
  }, [map]);

  useLayoutEffect(() => {
    const element = tooltipRef.current;
    if (!element || !state) return;
    const rect = element.getBoundingClientRect();
    setTooltipSize({ width: rect.width, height: rect.height });
  }, [state]);

  if (!state || viewport.width <= 0 || viewport.height <= 0) return null;
  const position = constrainMapTooltipPosition(state.anchor, viewport, tooltipSize, 8);
  return (
    <div
      ref={tooltipRef}
      role="tooltip"
      data-aoi-boundary-tooltip
      className="pointer-events-none absolute z-[690] min-w-[160px] max-w-[240px] rounded-lg border border-teal-200 bg-white/96 px-2.5 py-2 text-xs text-slate-900 shadow-lg dark:border-teal-800 dark:bg-slate-950/96 dark:text-slate-100"
      style={{ left: position.x, top: position.y }}
    >
      <div className="truncate font-semibold" title={name}>{name}</div>
      <div className="mt-0.5 text-muted-foreground">{kind}</div>
      {areaKm2 > 0 && (
        <div className="mt-0.5 font-mono text-[11px]">
          {t("panels.map.areaApprox", { area: areaKm2.toLocaleString(undefined, { maximumFractionDigits: areaKm2 < 100 ? 1 : 0 }) })}
        </div>
      )}
    </div>
  );
}

function FitToData({
  bbox,
  aoiFocusBbox,
  lockViewportOnSceneChange = false,
  focusBoundsRequest = null,
}: {
  bbox: Bbox;
  aoiFocusBbox?: Bbox | null;
  lockViewportOnSceneChange?: boolean;
  focusBoundsRequest?: { bbox: Bbox; nonce: number } | null;
}) {
  const map = useMap();
  const prevAoiKeyRef = useRef<string | null>(null);
  const isFirstAoiRenderRef = useRef<boolean>(true);
  const hasInitialFitRef = useRef<boolean>(false);
  const lastFocusNonceRef = useRef<number | null>(null);

  useEffect(() => {
    if (hasInitialFitRef.current && lockViewportOnSceneChange) {
      return;
    }
    hasInitialFitRef.current = true;
    map.fitBounds(boundsFromBbox(bbox), {
      animate: false,
      maxZoom: 12,
      padding: [36, 36],
    });
    // fitBounds with animate:false does not always emit moveend; the centre readout listens for it.
    // The interaction-test map double has no fire().
    if (typeof (map as { fire?: unknown }).fire === "function") {
      map.fire("moveend");
    }
    const center = map.getCenter();
    defaultMapFitReporter.reportFit({
      center: { lat: center.lat, lng: center.lng },
      zoom: map.getZoom(),
      bbox,
    });
  }, [bbox.east, bbox.north, bbox.south, bbox.west, lockViewportOnSceneChange, map]);

  useEffect(() => {
    const nextKey = aoiFocusBbox
      ? `${aoiFocusBbox.west.toFixed(5)}:${aoiFocusBbox.south.toFixed(5)}:${aoiFocusBbox.east.toFixed(5)}:${aoiFocusBbox.north.toFixed(5)}`
      : null;
    const prevKey = prevAoiKeyRef.current;
    prevAoiKeyRef.current = nextKey;
    if (isFirstAoiRenderRef.current) {
      isFirstAoiRenderRef.current = false;
      return;
    }
    if (!aoiFocusBbox || !nextKey || prevKey === nextKey) return;
    map.fitBounds(boundsFromBbox(aoiFocusBbox), {
      animate: false,
      maxZoom: 12,
      padding: [36, 36],
    });
    if (typeof (map as { fire?: unknown }).fire === "function") {
      map.fire("moveend");
    }
    const center = map.getCenter();
    defaultMapFitReporter.reportFit({
      center: { lat: center.lat, lng: center.lng },
      zoom: map.getZoom(),
      bbox: aoiFocusBbox,
    });
  }, [aoiFocusBbox, map]);

  useEffect(() => {
    if (!focusBoundsRequest || focusBoundsRequest.nonce === lastFocusNonceRef.current) return;
    lastFocusNonceRef.current = focusBoundsRequest.nonce;
    map.fitBounds(boundsFromBbox(focusBoundsRequest.bbox), {
      padding: [36, 36],
      maxZoom: 13,
    });
    if (typeof (map as { fire?: unknown }).fire === "function") {
      map.fire("moveend");
    }
    const center = map.getCenter();
    defaultMapFitReporter.reportFit({
      center: { lat: center.lat, lng: center.lng },
      zoom: map.getZoom(),
      bbox: focusBoundsRequest.bbox,
    });
  }, [focusBoundsRequest, map]);

  return null;
}

function MapApiBridge({ onReady }: { onReady: (map: LeafletMap | null) => void }) {
  const map = useMap();

  useEffect(() => {
    onReady(map);
    markUiSmoke("leaflet_initialized", true);
    const rect = map.getContainer().getBoundingClientRect();
    markUiSmoke("map_width", Math.round(rect.width));
    markUiSmoke("map_height", Math.round(rect.height));
    return () => onReady(null);
  }, [map, onReady]);

  return null;
}

function ResizeMapWithContainer() {
  const map = useMap();

  useEffect(() => {
    if (typeof ResizeObserver === "undefined") return;
    const container = map.getContainer();
    const schedule =
      typeof window.requestAnimationFrame === "function"
        ? window.requestAnimationFrame.bind(window)
        : (cb: FrameRequestCallback) => window.setTimeout(() => cb(Date.now()), 0);
    const cancel =
      typeof window.cancelAnimationFrame === "function"
        ? window.cancelAnimationFrame.bind(window)
        : (handle: number) => window.clearTimeout(handle);
    let frame = 0;
    const invalidate = () => {
      cancel(frame);
      frame = schedule(() => {
        map.invalidateSize({ animate: false });
        const rect = container.getBoundingClientRect();
        markUiSmoke("map_width", Math.round(rect.width));
        markUiSmoke("map_height", Math.round(rect.height));
        markUiSmoke("map_resized", true);
      });
    };
    const observer = new ResizeObserver(invalidate);
    observer.observe(container);
    invalidate();
    return () => {
      observer.disconnect();
      cancel(frame);
    };
  }, [map]);

  return null;
}

function MapToolButton({
  active,
  disabled = false,
  children,
  onClick,
  onHover,
  onFocusButton,
  onBlurButton,
  dataTestId,
  ariaDescribedBy,
  title,
  declutter = false,
}: {
  active?: boolean;
  disabled?: boolean;
  children: ReactNode;
  onClick?: () => void;
  onHover?: (el: HTMLButtonElement) => void;
  onFocusButton?: (el: HTMLButtonElement) => void;
  onBlurButton?: () => void;
  dataTestId?: string;
  ariaDescribedBy?: string;
  title: string;
  /** Agent 侧栏面板用的简洁样式；经典工作台保持原样 */
  declutter?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      aria-disabled={disabled ? "true" : undefined}
      data-testid={dataTestId}
      title={declutter ? undefined : title}
      aria-label={title}
      aria-describedby={ariaDescribedBy}
      onMouseEnter={(event) => onHover?.(event.currentTarget)}
      onFocus={(event) => onFocusButton?.(event.currentTarget)}
      onBlur={() => onBlurButton?.()}
      onPointerDown={(event) => event.stopPropagation()}
      onPointerUp={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
      onMouseUp={(event) => event.stopPropagation()}
      onDoubleClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (disabled) return;
        onClick?.();
      }}
      className={
        declutter
          ? cn(
              "w-8 h-8 rounded-sp-sm flex items-center justify-center text-sp-text-secondary transition-colors duration-fast ease-sp",
              disabled
                ? "opacity-40 cursor-not-allowed"
                : "hover:text-sp-text hover:bg-sp-hover cursor-pointer",
              active && !disabled && "bg-sp-selected text-sp-text",
            )
          : cn(
              "flex h-9 w-9 items-center justify-center border-b border-sp-line bg-sp-surface text-sp-text transition-colors duration-fast ease-sp last:border-b-0 dark:bg-sp-card",
              disabled
                ? "opacity-40 cursor-not-allowed"
                : "hover:bg-sp-hover cursor-pointer",
              active && !disabled && "bg-primary text-primary-foreground hover:bg-primary dark:bg-primary",
            )
      }
    >
      {children}
    </button>
  );
}

function MapToolbar({
  map,
  drawMode,
  drawActive,
  drawDisabled = false,
  layersOpen,
  onDrawModeChange,
  onDrawActiveChange,
  onClearLayers,
  onToggleLayers,
  declutter = false,
}: {
  declutter?: boolean;
  map: LeafletMap | null;
  drawMode: WorkbenchDrawMode;
  drawActive: boolean;
  drawDisabled?: boolean;
  layersOpen: boolean;
  onDrawModeChange: (mode: WorkbenchDrawMode) => void;
  onDrawActiveChange: (active: boolean) => void;
  onClearLayers: () => void;
  onToggleLayers: () => void;
}) {
  const { t } = useI18n();
  const toolbarRef = useRef<HTMLDivElement | null>(null);
  const tooltipId = useId();
  const [fluidTooltip, setFluidTooltip] = useState<{
    label: string;
    y: number;
    visible: boolean;
    warm: boolean;
    activeIndex: number | null;
  }>({
    label: "",
    y: 4,
    visible: false,
    warm: false,
    activeIndex: null,
  });

  const updateTooltipForButton = (btn: HTMLButtonElement, label: string, fallbackIndex: number) => {
    const rawTop = btn.offsetTop;
    const rawHeight = btn.offsetHeight;
    const y = rawHeight > 0 ? Math.round(rawTop + (rawHeight - 26) / 2) : 4 + fallbackIndex * 36;
    setFluidTooltip((prev) => ({
      label,
      y,
      visible: true,
      warm: prev.visible,
      activeIndex: fallbackIndex,
    }));
  };

  const hideFluidTooltip = () => {
    setFluidTooltip((prev) =>
      prev.visible ? { ...prev, visible: false, warm: false, activeIndex: null } : prev,
    );
  };

  const activate = (mode: WorkbenchDrawMode) => {
    if (drawActive && drawMode === mode) {
      onDrawActiveChange(false);
      return;
    }
    onDrawModeChange(mode);
    onDrawActiveChange(true);
  };

  useEffect(() => {
    if (!toolbarRef.current) return;
    DomEvent.disableClickPropagation(toolbarRef.current);
    DomEvent.disableScrollPropagation(toolbarRef.current);
  }, []);

  return (
    <div
      ref={toolbarRef}
      data-testid="map-toolbar"
      className={
        declutter
          ? "pointer-events-auto absolute left-3 top-3 z-[760] flex flex-col gap-1 bg-sp-surface rounded-sp-md shadow-overlay p-1"
          : "pointer-events-auto absolute left-4 top-4 z-[760] flex flex-col gap-3"
      }
      onMouseLeave={hideFluidTooltip}
      onPointerDown={(event) => event.stopPropagation()}
      onPointerUp={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
      onMouseUp={(event) => event.stopPropagation()}
      onMouseMove={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      {declutter ? (
        <>
          <MapToolButton
            declutter
            dataTestId="map-tool-btn-zoom-in"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 0 ? tooltipId : undefined}
            title={t("panels.map.zoomIn")}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.zoomIn"), 0)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.zoomIn"), 0)}
            onBlurButton={hideFluidTooltip}
            onClick={() => map?.zoomIn()}
          >
            <Plus className="h-4 w-4" />
          </MapToolButton>
          <MapToolButton
            declutter
            dataTestId="map-tool-btn-zoom-out"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 1 ? tooltipId : undefined}
            title={t("panels.map.zoomOut")}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.zoomOut"), 1)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.zoomOut"), 1)}
            onBlurButton={hideFluidTooltip}
            onClick={() => map?.zoomOut()}
          >
            <Minus className="h-4 w-4" />
          </MapToolButton>
          <MapToolButton
            declutter
            disabled={drawDisabled}
            dataTestId="map-tool-btn-draw-polygon"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 2 ? tooltipId : undefined}
            title={t("panels.map.drawPolygon")}
            active={!drawDisabled && drawActive && drawMode === "polygon"}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.drawPolygon"), 2)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.drawPolygon"), 2)}
            onBlurButton={hideFluidTooltip}
            onClick={() => activate("polygon")}
          >
            <Pentagon className="h-4 w-4" />
          </MapToolButton>
          <MapToolButton
            declutter
            disabled={drawDisabled}
            dataTestId="map-tool-btn-draw-rect"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 3 ? tooltipId : undefined}
            title={t("panels.map.drawRectangle")}
            active={!drawDisabled && drawActive && drawMode === "rect"}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.drawRectangle"), 3)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.drawRectangle"), 3)}
            onBlurButton={hideFluidTooltip}
            onClick={() => activate("rect")}
          >
            <Square className="h-4 w-4" />
          </MapToolButton>
          <MapToolButton
            declutter
            disabled={drawDisabled}
            dataTestId="map-tool-btn-clear"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 4 ? tooltipId : undefined}
            title={t("panels.map.clearLayers")}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.clearLayers"), 4)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.clearLayers"), 4)}
            onBlurButton={hideFluidTooltip}
            onClick={onClearLayers}
          >
            <Trash2 className="h-4 w-4" />
          </MapToolButton>
          <MapToolButton
            declutter
            dataTestId="map-tool-btn-layers"
            ariaDescribedBy={fluidTooltip.visible && fluidTooltip.activeIndex === 5 ? tooltipId : undefined}
            title={t("panels.map.switchBasemap")}
            active={layersOpen}
            onHover={(el) => updateTooltipForButton(el, t("panels.map.switchBasemap"), 5)}
            onFocusButton={(el) => updateTooltipForButton(el, t("panels.map.switchBasemap"), 5)}
            onBlurButton={hideFluidTooltip}
            onClick={onToggleLayers}
          >
            <Layers className="h-4 w-4" />
          </MapToolButton>
          <div
            id={tooltipId}
            role="tooltip"
            aria-hidden={!fluidTooltip.visible}
            data-testid="map-toolbar-fluid-tooltip"
            data-visible={fluidTooltip.visible ? "true" : "false"}
            data-warm={fluidTooltip.warm ? "true" : "false"}
            data-tooltip-label={fluidTooltip.label}
            className={cn("sp-fluid-tooltip", fluidTooltip.visible && "is-visible")}
            style={{ "--sp-tooltip-y": `${fluidTooltip.y}px` } as CSSProperties}
          >
            {fluidTooltip.label}
          </div>
        </>
      ) : (
        <>
          <div className="overflow-hidden rounded-sp-md border border-sp-line bg-sp-surface shadow-sp-card dark:bg-sp-card">
            <MapToolButton title={t("panels.map.zoomIn")} onClick={() => map?.zoomIn()}>
              <Plus className="h-4 w-4" />
            </MapToolButton>
            <MapToolButton title={t("panels.map.zoomOut")} onClick={() => map?.zoomOut()}>
              <Minus className="h-4 w-4" />
            </MapToolButton>
          </div>
          <div className="overflow-hidden rounded-sp-md border border-sp-line bg-sp-surface shadow-sp-card dark:bg-sp-card">
            <MapToolButton
              disabled={drawDisabled}
              title={t("panels.map.drawPolygon")}
              active={!drawDisabled && drawActive && drawMode === "polygon"}
              onClick={() => activate("polygon")}
            >
              <Pentagon className="h-4 w-4" />
            </MapToolButton>
            <MapToolButton
              disabled={drawDisabled}
              title={t("panels.map.drawRectangle")}
              active={!drawDisabled && drawActive && drawMode === "rect"}
              onClick={() => activate("rect")}
            >
              <Square className="h-4 w-4" />
            </MapToolButton>
          </div>
          <div className="overflow-hidden rounded-sp-md border border-sp-line bg-sp-surface shadow-sp-card dark:bg-sp-card">
            <MapToolButton disabled={drawDisabled} title={t("panels.map.clearLayers")} onClick={onClearLayers}>
              <Trash2 className="h-4 w-4" />
            </MapToolButton>
          </div>
          <div className="overflow-hidden rounded-sp-md border border-sp-line bg-sp-surface shadow-sp-card dark:bg-sp-card">
            <MapToolButton title={t("panels.map.switchBasemap")} active={layersOpen} onClick={onToggleLayers}>
              <Layers className="h-4 w-4" />
            </MapToolButton>
          </div>
        </>
      )}
    </div>
  );
}

function DrawLayer({
  mode,
  active,
  onRect,
  onPolygon,
  onPoint,
}: {
  mode: WorkbenchDrawMode;
  active: boolean;
  onRect: (bbox: Bbox) => void;
  onPolygon: (ring: [number, number][]) => void;
  onPoint: (lat: number, lng: number) => void;
}) {
  const map = useMap();
  const [start, setStart] = useState<LatLng | null>(null);
  const [current, setCurrent] = useState<LatLng | null>(null);
  const [sketch, setSketch] = useState<LatLng[]>([]);
  const [hover, setHover] = useState<LatLng | null>(null);
  const [lastPoint, setLastPoint] = useState<LatLng | null>(null);

  const isFinishedRef = useRef(false);
  const startRef = useRef<LatLng | null>(null);
  const currentRef = useRef<LatLng | null>(null);
  startRef.current = start;
  currentRef.current = current;

  const appendUniquePoint = useCallback((points: LatLng[], point: LatLng) => {
    const last = points[points.length - 1];
    if (last && last.distanceTo(point) < 0.5) return points;
    return [...points, point];
  }, []);

  const finishPolygon = useCallback(
    (extra?: LatLng) => {
      const points = extra ? appendUniquePoint(sketch, extra) : sketch;
      if (points.length < 3) {
        setSketch([]);
        setHover(null);
        return;
      }
      const ring = canonicalizeLeafletPolygonRing(points.map((point) => [point.lng, point.lat]));
      if (!ring) {
        setSketch([]);
        setHover(null);
        return;
      }
      onPolygon(ring);
      setSketch([]);
      setHover(null);
    },
    [appendUniquePoint, onPolygon, sketch],
  );

  useEffect(() => {
    if (!active) return;
    const dragWasEnabled = map.dragging.enabled();
    const dblClickWasEnabled = map.doubleClickZoom.enabled();
    if (mode === "rect" || mode === "polygon") map.dragging.disable();
    if (mode === "polygon") map.doubleClickZoom.disable();
    return () => {
      if (dragWasEnabled) map.dragging.enable();
      else map.dragging.disable();
      if (dblClickWasEnabled) map.doubleClickZoom.enable();
      else map.doubleClickZoom.disable();
    };
  }, [active, map, mode]);

  const finishRect = useCallback(
    (lastPos?: LatLng | null) => {
      if (isFinishedRef.current) return;
      isFinishedRef.current = true;
      const s = startRef.current;
      const c = lastPos ?? currentRef.current;
      if (s && c) {
        const authoritative = canonicalizeLeafletRectangle(
          [s.lng, s.lat],
          [c.lng, c.lat],
        );
        if (authoritative?.kind === "bbox") onRect(authoritative.bbox);
        if (authoritative?.kind === "polygon") onPolygon(authoritative.ring);
      }
      setStart(null);
      setCurrent(null);
    },
    [onPolygon, onRect],
  );

  useEffect(() => {
    isFinishedRef.current = false;
    setStart(null);
    setCurrent(null);
    setSketch([]);
    setHover(null);
  }, [mode]);

  useEffect(() => {
    if (!active || mode !== "polygon") return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Enter") finishPolygon();
      if (event.key === "Escape") {
        setSketch([]);
        setHover(null);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, finishPolygon, mode]);

  useEffect(() => {
    if (!active || mode !== "rect" || !start) return;

    const handleWindowPointerUp = () => {
      finishRect(currentRef.current);
    };

    const handleWindowKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        isFinishedRef.current = true;
        setStart(null);
        setCurrent(null);
      }
    };

    window.addEventListener("pointerup", handleWindowPointerUp);
    window.addEventListener("mouseup", handleWindowPointerUp);
    window.addEventListener("keydown", handleWindowKeyDown);

    return () => {
      window.removeEventListener("pointerup", handleWindowPointerUp);
      window.removeEventListener("mouseup", handleWindowPointerUp);
      window.removeEventListener("keydown", handleWindowKeyDown);
    };
  }, [active, finishRect, mode, start]);

  useMapEvents({
    mousedown(e) {
      if (!active || mode !== "rect") return;
      e.originalEvent.preventDefault();
      isFinishedRef.current = false;
      setStart(e.latlng);
      setCurrent(e.latlng);
    },
    mousemove(e) {
      if (!active) return;
      if (mode === "rect" && start) setCurrent(e.latlng);
      if (mode === "polygon" && sketch.length) setHover(e.latlng);
    },
    mouseup() {
      if (!active || mode !== "rect") return;
      finishRect(currentRef.current);
    },
    click(e: LeafletMouseEvent) {
      if (!active) return;
      if (mode === "point") {
        setLastPoint(e.latlng);
        const longitude = canonicalLeafletLongitude(e.latlng.lng);
        if (longitude !== null) onPoint(e.latlng.lat, longitude);
      }
      if (mode === "polygon") {
        setSketch((prev) => appendUniquePoint(prev, e.latlng));
        setHover(null);
      }
    },
    dblclick(e: LeafletMouseEvent) {
      if (!active || mode !== "polygon") return;
      e.originalEvent.preventDefault();
      finishPolygon(e.latlng);
    },
  });

  if (mode === "rect" && start && current) {
    const south = Math.min(start.lat, current.lat);
    const north = Math.max(start.lat, current.lat);
    const west = Math.min(start.lng, current.lng);
    const east = Math.max(start.lng, current.lng);
    return (
      <Rectangle
        bounds={[
          [south, west],
          [north, east],
        ]}
        pathOptions={{ color: "#22c55e", weight: 2, dashArray: "6 4", fillOpacity: 0.12 }}
      />
    );
  }

  const polygonPreview = hover ? [...sketch, hover] : sketch;
  return (
    <>
      {mode === "polygon" && polygonPreview.length > 1 && (
        <Polyline
          positions={polygonPreview.map((p) => [p.lat, p.lng])}
          pathOptions={{ color: "#22c55e", weight: 2.5 }}
        />
      )}
      {lastPoint && (
        <CircleMarker
          center={[lastPoint.lat, lastPoint.lng]}
          radius={6}
          pathOptions={{ color: "#22c55e", fillColor: "#22c55e", fillOpacity: 0.85 }}
        />
      )}
    </>
  );
}

function MapStatusOverlay({
  chrome = "default",
  declutter = false,
  mapStatus = "external_tiles_ready",
  containerWidth = 1000,
}: {
  chrome?: "default" | "panel";
  declutter?: boolean;
  mapStatus?: MapRuntimeStatus;
  containerWidth?: number;
}) {
  const { t } = useI18n();
  if (chrome === "panel" && !declutter) return null;
  const isNarrow = typeof containerWidth === "number" && containerWidth > 0 && containerWidth < 420;
  if (declutter && isNarrow) return null;
  const map = useMap();
  const [status, setStatus] = useState(() => {
    const center = map.getCenter();
    return { lat: center.lat, lng: center.lng, zoom: map.getZoom(), label: t("panels.map.coordinateCenter") };
  });
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    lat: number;
    lng: number;
  } | null>(null);
  const contextMenuRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const el = contextMenuRef.current;
    if (!el) return;
    DomEvent.disableClickPropagation(el);
    DomEvent.disableScrollPropagation(el);
  }, [contextMenu]);

  useEffect(() => {
    if (!contextMenu) return;
    const onPointerDown = (e: MouseEvent | TouchEvent) => {
      if (contextMenuRef.current && !contextMenuRef.current.contains(e.target as Node)) {
        setContextMenu(null);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setContextMenu(null);
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [contextMenu]);
  const pointerFrameRef = useRef<number | null>(null);
  const pendingPointerStatusRef = useRef<{ lat: number; lng: number; zoom: number; label: string } | null>(null);

  const cancelPendingPointerStatus = useCallback(() => {
    pendingPointerStatusRef.current = null;
    if (pointerFrameRef.current !== null) {
      window.cancelAnimationFrame(pointerFrameRef.current);
      pointerFrameRef.current = null;
    }
  }, []);
  const schedulePointerStatus = useCallback((lat: number, lng: number) => {
    pendingPointerStatusRef.current = { lat, lng, zoom: map.getZoom(), label: t("panels.map.coordinatePointer") };
    if (pointerFrameRef.current !== null) return;
    pointerFrameRef.current = window.requestAnimationFrame(() => {
      pointerFrameRef.current = null;
      const pending = pendingPointerStatusRef.current;
      pendingPointerStatusRef.current = null;
      if (pending) setStatus(pending);
    });
  }, [map, t]);
  const updateFromCenter = useCallback(
    (label = t("panels.map.coordinateCenter")) => {
      cancelPendingPointerStatus();
      const center = map.getCenter();
      setStatus({ lat: center.lat, lng: center.lng, zoom: map.getZoom(), label });
    },
    [cancelPendingPointerStatus, map, t],
  );

  useEffect(() => {
    updateFromCenter();
    return cancelPendingPointerStatus;
  }, [cancelPendingPointerStatus, updateFromCenter]);

  useEffect(() => {
    const unsubscribe = defaultMapFitReporter.subscribe((fitView) => {
      cancelPendingPointerStatus();
      setStatus({
        lat: fitView.center.lat,
        lng: fitView.center.lng,
        zoom: fitView.zoom,
        label: t("panels.map.coordinateCenter"),
      });
    });
    const latest = defaultMapFitReporter.getLatest();
    if (latest) {
      cancelPendingPointerStatus();
      setStatus({
        lat: latest.center.lat,
        lng: latest.center.lng,
        zoom: latest.zoom,
        label: t("panels.map.coordinateCenter"),
      });
    }
    return () => {
      unsubscribe();
    };
  }, [cancelPendingPointerStatus, t]);

  useMapEvents({
    mousemove(e) {
      schedulePointerStatus(e.latlng.lat, e.latlng.lng);
    },
    mouseout() {
      updateFromCenter();
    },
    movestart() {
      setContextMenu(null);
    },
    zoomstart() {
      setContextMenu(null);
    },
    contextmenu(e) {
      e.originalEvent?.preventDefault?.();
      const lng = canonicalLeafletLongitude(e.latlng.lng) ?? e.latlng.lng;
      const size = map.getSize();
      const menuW = 242;
      const menuH = 112;
      const x = Math.max(8, Math.min(e.containerPoint.x, Math.max(8, size.x - menuW - 8)));
      const y = Math.max(8, Math.min(e.containerPoint.y, Math.max(8, size.y - menuH - 8)));
      setContextMenu({ x, y, lat: e.latlng.lat, lng });
    },
    moveend() {
      updateFromCenter(status.label === t("panels.map.coordinatePointer") ? t("panels.map.coordinateCenter") : status.label);
    },
    zoomend() {
      updateFromCenter(status.label === t("panels.map.coordinatePointer") ? t("panels.map.coordinateCenter") : status.label);
    },
    viewreset() {
      updateFromCenter(status.label === t("panels.map.coordinatePointer") ? t("panels.map.coordinateCenter") : status.label);
    },
  });

  const statusMessages: Record<MapRuntimeStatus, string> = {
    map_initializing: t("panels.map.statusMapInitializing"),
    external_tiles_loading: t("panels.map.statusTilesLoading"),
    external_tiles_ready: t("panels.map.statusTilesReady"),
    external_tiles_failed: t("panels.map.statusTilesFailed"),
    csp_or_network_blocked: t("panels.map.statusBlocked"),
  };

  const dotColor =
    mapStatus === "external_tiles_ready"
      ? "bg-sp-success-text"
      : mapStatus === "external_tiles_failed" || mapStatus === "csp_or_network_blocked"
      ? "bg-sp-danger-text"
      : "bg-sp-accent animate-pulse";

  const contextMenuOverlay = contextMenu ? (
    <div
      ref={contextMenuRef}
      data-testid="map-context-menu"
      style={{ left: contextMenu.x, top: contextMenu.y, zIndex: 760 }}
      className="pointer-events-auto absolute w-[238px] rounded-sp-md border border-sp-line bg-sp-card/95 p-1.5 text-xs text-sp-text shadow-overlay backdrop-blur-xl select-none"
      onPointerDown={(ev) => ev.stopPropagation()}
      onMouseDown={(ev) => ev.stopPropagation()}
      onClick={(ev) => ev.stopPropagation()}
      onDoubleClick={(ev) => ev.stopPropagation()}
      onContextMenu={(ev) => {
        ev.preventDefault();
        ev.stopPropagation();
      }}
    >
      <div className="flex items-center justify-between gap-2 px-2 py-1 border-b border-sp-line/70 whitespace-nowrap">
        <span className="text-[11px] text-sp-text-secondary shrink-0">{t("panels.map.contextLonLat")}</span>
        <span data-testid="map-context-coords" className="font-mono tabular-nums text-[11px] font-semibold text-sp-text">
          {fmt(contextMenu.lng)}, {fmt(contextMenu.lat)}
        </span>
      </div>
      <div className="mt-1 flex flex-col gap-0.5">
        <button
          type="button"
          data-testid="map-context-copy-coords"
          onClick={(ev) => {
            ev.stopPropagation();
            const text = `${fmt(contextMenu.lng)}, ${fmt(contextMenu.lat)}`;
            try {
              if (navigator.clipboard?.writeText) {
                void navigator.clipboard.writeText(text);
              } else {
                const ta = document.createElement("textarea");
                ta.value = text;
                ta.style.position = "fixed";
                ta.style.opacity = "0";
                document.body.appendChild(ta);
                ta.select();
                document.execCommand("copy");
                document.body.removeChild(ta);
              }
            } catch {
              // ignore clipboard error
            }
            emitTopToast({ text: t("panels.map.contextCopied", { text }), tone: "success" });
            setContextMenu(null);
          }}
          className="w-full rounded-sp-sm px-2 py-1.5 text-left text-xs text-sp-text hover:bg-sp-hover cursor-pointer transition-colors"
        >
          {t("panels.map.contextCopyCoords")}
        </button>
        <button
          type="button"
          data-testid="map-context-center-here"
          onClick={(ev) => {
            ev.stopPropagation();
            const targetLat = contextMenu.lat;
            const targetLng = contextMenu.lng;
            setContextMenu(null);
            const prefersReducedMotion =
              typeof window !== "undefined"
              && Boolean(window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches);
            map.setView([targetLat, targetLng], map.getZoom(), { animate: !prefersReducedMotion });
            setStatus({
              label: t("panels.map.coordinateCenter"),
              lat: targetLat,
              lng: targetLng,
              zoom: map.getZoom(),
            });
          }}
          className="w-full rounded-sp-sm px-2 py-1.5 text-left text-xs text-sp-text hover:bg-sp-hover cursor-pointer transition-colors"
        >
          {t("panels.map.contextCenterHere")}
        </button>
      </div>
    </div>
  ) : null;

  if (declutter) {
    return (
      <>
        <div
          data-testid="map-coordinate-overlay"
          data-map-status={mapStatus}
          className={cn(
            MAP_CHIP_BASE_CLASS,
            "pointer-events-none absolute left-3 bottom-3 z-[520] flex max-w-[calc(100%-3rem)] items-center gap-2 select-none",
          )}
        >
          <span
            className={cn("w-[6px] h-[6px] rounded-full shrink-0", dotColor)}
            title={statusMessages[mapStatus] || t("panels.map.basemapStatus")}
          />
          <span className="shrink-0 text-sp-text-secondary">{status.label}</span>
          <span className="min-w-0 truncate font-mono tabular-nums text-sp-text">
            {fmt(canonicalLeafletLongitude(status.lng) ?? status.lng)}, {fmt(status.lat)} · Z{status.zoom}
          </span>
        </div>
        {contextMenuOverlay}
      </>
    );
  }

  return (
    <>
      <div
        data-testid="map-coordinate-overlay"
        className="pointer-events-none absolute left-3 z-[520] max-w-[calc(100%-3rem)] truncate rounded-sp-sm border border-sp-line bg-sp-surface/95 px-2.5 py-1 text-[11px] text-sp-text shadow-overlay"
        style={{
          bottom: "max(1rem, calc(env(safe-area-inset-bottom) + 1rem))",
        }}
      >
        <span className="text-sp-text-secondary">{status.label}</span>
        <span className="ml-1.5 font-mono tabular-nums">
          {fmt(canonicalLeafletLongitude(status.lng) ?? status.lng)}, {fmt(status.lat)} · Z{status.zoom}
        </span>
      </div>
      {contextMenuOverlay}
    </>
  );
}

function MapProviderStatusOverlay({
  status,
  chrome = "default",
  declutter = false,
}: {
  status: MapRuntimeStatus;
  chrome?: "default" | "panel";
  declutter?: boolean;
}) {
  const { t } = useI18n();
  // 底图加载完成后立即自动隐藏；异常状态照常醒目提示
  if (status === "external_tiles_ready") {
    return null;
  }
  const messages: Record<MapRuntimeStatus, string> = {
    map_initializing: t("panels.map.statusMapInitializing"),
    external_tiles_loading: t("panels.map.statusTilesLoading"),
    external_tiles_ready: t("panels.map.statusTilesReady"),
    external_tiles_failed: t("panels.map.statusTilesFailed"),
    csp_or_network_blocked: t("panels.map.statusBlocked"),
  };
  const blocked = status === "external_tiles_failed" || status === "csp_or_network_blocked";
  return (
    <div
      data-map-status={status}
      className={cn(
        "absolute left-3 z-[520] max-w-[min(28rem,calc(100%-1.5rem))] rounded-sp-sm border border-sp-line/80 bg-sp-surface/85 backdrop-blur-md px-2 py-1 text-[11px] shadow-overlay select-none transition-opacity",
        blocked
          ? "pointer-events-auto text-sp-text"
          : "pointer-events-none text-sp-text-secondary",
        declutter || chrome === "panel" ? "bottom-11" : "",
      )}
      style={
        declutter || chrome === "panel"
          ? undefined
          : { bottom: "max(3.25rem, calc(env(safe-area-inset-bottom) + 3.25rem))" }
      }
      role={blocked ? "alert" : "status"}
      aria-live="polite"
    >
      <div className="flex items-center gap-1.5">
        {blocked ? (
          <LocateFixed className="h-3 w-3 shrink-0 text-amber-600" />
        ) : (
          <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-sp-text-muted animate-pulse" aria-hidden="true" />
        )}
        <span className="whitespace-nowrap">{messages[status]}</span>
      </div>
    </div>
  );
}

function UiSmokeMapChecks({ aoiOverlayRendered }: { aoiOverlayRendered: boolean }) {
  const map = useMap();

  useEffect(() => {
    if (!isUiSmokeMode()) return;
    const rect = map.getContainer().getBoundingClientRect();
    markUiSmoke("map_width", Math.round(rect.width));
    markUiSmoke("map_height", Math.round(rect.height));
    markUiSmoke("aoi_overlay_rendered", aoiOverlayRendered);
  }, [aoiOverlayRendered, map]);

  if (!isUiSmokeMode()) return null;
  return (
    <TileLayer
      key="ui-smoke-local-tile-error"
      url="./__insar_studio_ui_smoke_missing_tile__.svg"
      opacity={0}
      eventHandlers={{ tileerror: () => markUiSmoke("tile_error_handled", true) }}
    />
  );
}

export interface WorkbenchMapProps {
  bbox?: Bbox | null;
  aoiBbox?: Bbox | null;
  aoiGeometry?: Json | null;
  aoiName?: string;
  aoiKind?: string;
  /** Rectangle-only DEM acquisition extent derived from the authoritative AOI bbox. */
  demEnvelopeBbox?: Bbox | null;
  /** Native-derived render-only authority for the persisted AOI, when available. */
  aoiDisplayAuthority?: CoreAoiDisplayAuthorityProjection | null;
  sceneBbox?: Bbox | null;
  /** Search-result extents shown while exact administrative geometry is being resolved. */
  candidateBboxes?: Bbox[];
  scenes?: SceneRow[];
  selectedSceneId?: string | null;
  hoveredSceneId?: string | null;
  stackSizeFor?: (sceneId: string) => number;
  stackScenesFor?: (sceneId: string) => SceneRow[];
  preview?: PreviewRuntimeState;
  previewTileLayer?: LandsatMapTilePreview | Sentinel2MapTilePreview | null;
  showTileStatusOverlay?: boolean;
  activePreviewProductKey?: string | null;
  onPreviewTileStatusChange?: (event: {
    productKey: string;
    identity: string;
    status: "loading" | "ready" | "failed";
    loadedCount: number;
    errorCount: number;
  }) => void;
  previewBbox?: Bbox | null;
  previewRenderBbox?: Bbox | null;
  previewGeometry?: Json | null;
  /** Native page render copy; never written back to previewGeometry. */
  previewRenderGeometry?: Json | null;
  /** Explicit multi-scene pinned previews toggled by row `预览` buttons (renders at opacity 1.0 clipped to footprints). */
  pinnedPreviews?: readonly PinnedMapPreviewEntry[];
  /** Invoked when a pinned raster preview fails to load in the map overlay. */
  onPinnedPreviewError?: (sceneId: string) => void;
  /** Undefined is legacy/no authority; an object is an identity-bound native result, including explicit nulls. */
  previewRenderAuthority?: PreviewRenderAuthority;
  layerKey: MapLayerKey;
  tiandituToken?: string;
  drawMode: WorkbenchDrawMode;
  drawActive: boolean;
  drawDisabled?: boolean;
  onLayerChange: (key: MapLayerKey) => void;
  onOpenTileDownloadWorkbench?: (suggestedTileSource: string) => void;
  onSceneSelect?: (sceneId: string) => void;
  onDrawModeChange: (mode: WorkbenchDrawMode) => void;
  onDrawActiveChange: (active: boolean) => void;
  onClearLayers: () => void;
  onRectDraw: (bbox: Bbox) => void;
  onPolygonDraw: (ring: [number, number][]) => void;
  onPointDraw: (lat: number, lng: number) => void;
  strictWorkspaceAoi?: boolean;
  orbitLabels?: { ascending: string; descending: string; unknown: string; heading?: string };
  chrome?: "default" | "panel";
  declutter?: boolean;
  fitScope?: "aoi" | "aoi-and-results";
  /** 足迹弹窗标题；默认 scene_id 保持旧界面不变 */
  popupTitleMode?: ScenePopupTitleMode;
  /** 隐藏足迹弹窗的「下载入口」行；默认显示（旧界面不变） */
  hideDownloadEntry?: boolean;
  /** Suppress built-in declutter legend dock when a specialized unified legend bar is passed in topControls. */
  hideDefaultLegend?: boolean;
  /** Optional commands beside the panel legend; absent in the classic workbench. */
  topControls?: React.ReactNode;
  /** Arrange top-right declutter controls (legend + topControls) in a single right-aligned vertical column. */
  verticalTopRightControls?: boolean;
  /** Vertical offset class for top-level map panel overlays (legend dock & stack popover) when a parent floating header occupies top-3. Defaults to "top-3". */
  topOverlayOffsetClass?: string;
  /** Disposable administrative candidate outline; never the working AOI. */
  adminPreviewGeometry?: Json | null;
  /** Real raster overlay (stitched XYZ mosaic PNG/GeoTIFF preview or GEE/XYZ tile layer). */
  geodataRasterOverlay?: GeodataRasterOverlaySpec | null;
  /** Keep current map zoom/center steady when scenes change across data-source switches. */
  lockViewportOnSceneChange?: boolean;
  /** Optional subset workspace candidate footprints. */
  subsetFootprintGeometries?: readonly Json[];
  /** Optional subset workspace highlighted preview scene footprint. */
  subsetPreviewGeometry?: Json | null;
  /** Optional subset workspace highlighted preview scene bbox for quicklook overlay. */
  subsetPreviewImageBbox?: Bbox | null;
  /** Optional subset workspace highlighted preview quicklook URL. */
  subsetPreviewImageUrl?: string | null;
  /** Optional subset workspace authoritative intersection polygons. */
  subsetEffectiveGeometries?: readonly Json[];
  /** Explicit one-shot fitBounds request keyed by nonce. */
  focusBoundsRequest?: { bbox: Bbox; nonce: number } | null;
  /** Controlled multi-temporal split-screen swipe left (Before) layer. */
  swipeLeftLayer?: MapSwipeLayerSpec | null;
  /** Controlled multi-temporal split-screen swipe right (After) layer. */
  swipeRightLayer?: MapSwipeLayerSpec | null;
  /** Controlled split-screen divider position percentage (2..98). */
  swipePosition?: number;
  /** Callback invoked when the split-screen divider moves. */
  onSwipePositionChange?: (position: number) => void;
  /** Optional callback to clear pinned split-screen swipe comparison. */
  onClearSwipe?: () => void;
  /** Hide floating mouseover metadata card over footprints (use result-row info button and stack card instead). */
  disableSceneHoverCard?: boolean;
  /** Optional callback invoked when a vector AOI file/bundle is dropped directly onto the map canvas. */
  onVectorAoiImported?: (geometry: Record<string, unknown>, bbox: Bbox, label: string) => void;
}

function WorkbenchMapComponent({
  bbox,
  aoiBbox,
  aoiGeometry,
  aoiName = translateNow("panels.map.customAoi"),
  aoiKind = translateNow("panels.map.customAoi"),
  demEnvelopeBbox = null,
  aoiDisplayAuthority = null,
  sceneBbox,
  candidateBboxes = [],
  scenes = [],
  selectedSceneId = null,
  hoveredSceneId = null,
  stackSizeFor,
  stackScenesFor,
  preview,
  previewTileLayer = null,
  activePreviewProductKey = null,
  onPreviewTileStatusChange,
  showTileStatusOverlay = true,
  previewBbox = null,
  previewRenderBbox = null,
  previewGeometry = null,
  previewRenderGeometry = null,
  previewRenderAuthority,
  pinnedPreviews,
  onPinnedPreviewError,
  layerKey,
  tiandituToken = "",
  drawMode,
  drawActive,
  onLayerChange,
  onOpenTileDownloadWorkbench,
  onSceneSelect,
  onDrawModeChange,
  onDrawActiveChange,
  onClearLayers,
  onRectDraw,
  onPolygonDraw,
  onPointDraw,
  strictWorkspaceAoi = false,
  orbitLabels,
  chrome = "default",
  declutter = false,
  fitScope = "aoi",
  popupTitleMode = "scene_id",
  hideDownloadEntry = false,
  hideDefaultLegend = false,
  drawDisabled = false,
  topControls,
  verticalTopRightControls = false,
  topOverlayOffsetClass = "top-3",
  adminPreviewGeometry = null,
  geodataRasterOverlay = null,
  lockViewportOnSceneChange = false,
  subsetFootprintGeometries = [],
  subsetPreviewGeometry = null,
  subsetPreviewImageBbox = null,
  subsetPreviewImageUrl = null,
  subsetEffectiveGeometries = [],
  focusBoundsRequest = null,
  swipeLeftLayer = null,
  swipeRightLayer = null,
  swipePosition = 50,
  onSwipePositionChange,
  onClearSwipe,
  disableSceneHoverCard = false,
  onVectorAoiImported,
}: WorkbenchMapProps) {
  const { t, locale } = useI18n();
  const smokeMode = isUiSmokeMode();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [htmlVectorDrag, setHtmlVectorDrag] = useState(false);
  const [nativeVectorDrag, setNativeVectorDrag] = useState(false);
  const [vectorImportBusy, setVectorImportBusy] = useState(false);
  const [vectorDropError, setVectorDropError] = useState<string | null>(null);
  const vectorDragDepthRef = useRef(0);

  const applyDroppedVectorResult = useCallback(
    (res: Awaited<ReturnType<typeof importVectorFromNativePaths>>) => {
      if (!res.ok) {
        setVectorDropError(res.error);
        return;
      }
      setVectorDropError(null);
      onVectorAoiImported?.(res.geometry, res.bbox, res.label);
    },
    [onVectorAoiImported],
  );

  useNativeVectorDropTarget({
    enabled: Boolean(onVectorAoiImported),
    priority: 10,
    onHoverChange: setNativeVectorDrag,
    onPathsDropped: (paths) => {
      if (!onVectorAoiImported) return;
      setVectorImportBusy(true);
      void importVectorFromNativePaths(paths, { persistWorkingAoi: true })
        .then(applyDroppedVectorResult)
        .finally(() => setVectorImportBusy(false));
    },
  });
  const [containerWidth, setContainerWidth] = useState<number>(1000);
  const [legendOpen, setLegendOpen] = useState(false);
  const legendPopoverRef = useRef<HTMLDivElement | null>(null);
  const legendButtonRef = useRef<HTMLButtonElement | null>(null);
  const legendTriggerLeftRef = useRef<HTMLSpanElement | null>(null);
  const legendGridContentRef = useRef<HTMLDivElement | null>(null);
  const [dockMetrics, setDockMetrics] = useState<{
    closedW: number;
    openW: number;
    openH: number;
  }>({ closedW: 112, openW: 192, openH: 82 });
  const [attributionOpen, setAttributionOpen] = useState(false);
  const [attributionHovered, setAttributionHovered] = useState(false);

  // 地图来源展开后 4 秒自动收起；鼠标停留时不收
  useEffect(() => {
    if (!attributionOpen || attributionHovered) return;
    const timer = window.setTimeout(() => setAttributionOpen(false), 4000);
    return () => window.clearTimeout(timer);
  }, [attributionOpen, attributionHovered]);
  const attributionRef = useRef<HTMLDivElement | null>(null);
  const attributionButtonRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!attributionOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAttributionOpen(false);
    };
    const onPointerDown = (e: MouseEvent | TouchEvent) => {
      if (
        attributionRef.current &&
        !attributionRef.current.contains(e.target as Node) &&
        attributionButtonRef.current &&
        !attributionButtonRef.current.contains(e.target as Node)
      ) {
        setAttributionOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [attributionOpen]);

  useEffect(() => {
    if (!containerRef.current) return;
    const updateWidth = () => {
      if (containerRef.current) {
        setContainerWidth(containerRef.current.clientWidth);
      }
    };
    updateWidth();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setContainerWidth(entry.contentRect.width);
      }
    });
    observer.observe(containerRef.current);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!legendOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setLegendOpen(false);
    };
    const onPointerDown = (e: MouseEvent | TouchEvent) => {
      if (
        legendPopoverRef.current &&
        !legendPopoverRef.current.contains(e.target as Node) &&
        legendButtonRef.current &&
        !legendButtonRef.current.contains(e.target as Node)
      ) {
        setLegendOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [legendOpen]);
  const effectiveToken = effectiveTiandituToken(tiandituToken);
  const hasTiandituToken = hasEffectiveTiandituToken(tiandituToken);
  const requestedLayerKey = isEnabledMapProvider(layerKey)
    ? layerKey
    : DEFAULT_MAP_PROVIDER.key;
  const requestedLayer = MAP_LAYERS[requestedLayerKey];
  const effectiveLayerKey = requestedLayer.requiresToken && !hasTiandituToken
    ? DEFAULT_MAP_PROVIDER.key
    : requestedLayerKey;
  const layer = MAP_LAYERS[effectiveLayerKey];
  const providerCrs = layer.crs;
  const [annotationPreference, setAnnotationPreference] = useState<AnnotationOverlayPreference>(
    DEFAULT_ANNOTATION_PREFERENCE,
  );
  const effectiveAnnotations = effectiveAnnotationOverlays(
    annotationPreference,
    layer.containsLabels,
  );
  const aoiEnvelope = useMemo(
    () => aoiDisplayAuthority?.envelope ?? (aoiGeometry ? deriveWrappedAoiEnvelope(aoiGeometry) : null),
    [aoiDisplayAuthority, aoiGeometry],
  );
  const unsupportedAoiProjection = Boolean(aoiGeometry && !aoiEnvelope && !aoiDisplayAuthority);
  const displayAoi = useMemo(
    () => (
      aoiDisplayAuthority?.render_geometry
        ?? (aoiGeometry && aoiEnvelope
          ? isFullWorldDisplayEnvelope(aoiEnvelope)
            ? aoiGeometry
            : shiftPolygonalGeojsonToDisplayDomain(aoiGeometry, aoiEnvelope)
          : null)
    ),
    [aoiDisplayAuthority, aoiEnvelope, aoiGeometry],
  );
  const renderedAoi = useMemo(
    () => displayAoi ? projectGeometryForMapProvider(displayAoi, providerCrs) : null,
    [displayAoi, providerCrs],
  );
  const displayCandidateBboxes = useMemo(
    () => candidateBboxes.flatMap((candidate) => {
      if (!isLocalDisplayBbox(candidate)) return [];
      const projected = aoiEnvelope
        ? shiftBboxToDisplayDomain(candidate, aoiEnvelope)
        : candidate;
      return isLocalDisplayBbox(projected)
        ? [projected]
        : [];
    }),
    [aoiEnvelope, candidateBboxes],
  );
  const renderedCandidateBboxes = useMemo(
    () => displayCandidateBboxes.map((candidate) =>
      projectBboxForMapProvider(candidate, providerCrs)),
    [displayCandidateBboxes, providerCrs],
  );
  const displayDemEnvelopeBbox = useMemo(() => {
    if (!demEnvelopeBbox) return null;
    const projected = aoiEnvelope
      ? shiftBboxToDisplayDomain(demEnvelopeBbox, aoiEnvelope)
      : demEnvelopeBbox;
    if (isLocalDisplayBbox(projected)) return projected;
    if (!aoiEnvelope) return null;
    const continuous = expandedDemDisplayBbox(aoiEnvelope, DEM_BBOX_BUFFER_DEGREES);
    return isLocalDisplayBbox(continuous) ? continuous : null;
  }, [aoiEnvelope, demEnvelopeBbox]);
  const renderedDemEnvelopeBbox = useMemo(
    () => displayDemEnvelopeBbox
      ? projectBboxForMapProvider(displayDemEnvelopeBbox, providerCrs)
      : null,
    [displayDemEnvelopeBbox, providerCrs],
  );
  const displayCandidateFocusBbox = useMemo(
    () => candidateViewBbox(displayCandidateBboxes),
    [displayCandidateBboxes],
  );
  const admittedSceneEntries = useMemo(
    () => scenes.flatMap((scene, index) => {
      const bboxFallback = !scene.footprint_geojson
        ? sceneFootprintFromBbox(scene.footprint_bbox)
        : null;
      const authoritativeFootprint = sceneFootprintRenderModel(scene.footprint_geojson) ?? bboxFallback;
      if (!authoritativeFootprint) return [];
      const canonicalGeometry = scene.footprint_geojson ?? bboxFallback?.geojson ?? null;
      const nativeAuthority = aoiDisplayAuthority &&
        scene.render_authority?.region_id === aoiDisplayAuthority.region_id &&
        scene.render_authority.revision === aoiDisplayAuthority.revision;
      const rawGlobalFallback = !aoiDisplayAuthority && isFullWorldDisplayEnvelope(aoiEnvelope);
      const requiresNativeSceneProjection = Boolean(
        aoiDisplayAuthority?.render_geometry
        && aoiDisplayAuthority.envelope.crossesAntimeridian
        && scene.footprint_geojson,
      );
      if (unsupportedAoiProjection) return [];
      const canonicalProjection = rawGlobalFallback
        ? canonicalGeometry
        : aoiEnvelope && canonicalGeometry
          ? shiftPolygonalGeojsonToLongitudeReference(canonicalGeometry, aoiEnvelope)
          : null;
      const renderedGeometry = aoiDisplayAuthority?.render_geometry
        ? nativeAuthority && scene.render_geometry
          ? scene.render_geometry
          : requiresNativeSceneProjection
            ? null
            : canonicalProjection
        : canonicalProjection;
      // Native antimeridian copies are required for authoritative GeoJSON. A
      // bbox-only legacy row has no seam geometry to preserve, so its shifted
      // render-only rectangle remains admissible.
      if (aoiEnvelope && scene.footprint_geojson && !renderedGeometry) return [];
      const footprint = renderedGeometry
        ? sceneFootprintRenderModel(canonicalGeometry, renderedGeometry)
        : (!aoiEnvelope && !aoiDisplayAuthority && !aoiGeometry ? authoritativeFootprint : null);
      return footprint ? [{
        scene,
        displayFootprint: footprint,
        footprint: projectSceneFootprintForMapProvider(footprint, providerCrs),
        index,
      }] : [];
    }),
    [aoiDisplayAuthority, aoiEnvelope, aoiGeometry, providerCrs, scenes, unsupportedAoiProjection],
  );
  const displaySceneUnion = useMemo(
    () => unionBbox(admittedSceneEntries.map((entry) => entry.displayFootprint.bbox)),
    [admittedSceneEntries],
  );
  const displayAoiFocusBbox = useMemo(() => {
    const projectedExplicitBbox = aoiEnvelope && isValidBbox(bbox)
      ? shiftBboxToDisplayDomain(bbox, aoiEnvelope)
      : bbox;
    return isLocalDisplayBbox(projectedExplicitBbox)
      ? projectedExplicitBbox
      : aoiEnvelope
      ? continuousDisplayBbox(aoiEnvelope)
      : isLocalDisplayBbox(aoiBbox)
      ? aoiBbox
      : displayCandidateFocusBbox ?? null;
  }, [aoiBbox, aoiEnvelope, bbox, displayCandidateFocusBbox]);
  const aoiFocusBbox = useMemo(
    () => displayAoiFocusBbox ? projectBboxForMapProvider(displayAoiFocusBbox, providerCrs) : null,
    [displayAoiFocusBbox, providerCrs],
  );
  const displayFitBbox = useMemo(() => {
    // Precedence is a contract (AOI-REG-003 / UI-REG-003): an explicit focus bbox, shifted into the
    // AOI's world copy, outranks the AOI envelope; only then the envelope, the AOI bbox, candidates.
    const baseAoiBbox = displayAoiFocusBbox;

    if (fitScope === "aoi-and-results") {
      const sceneUnion = isLocalDisplayBbox(displaySceneUnion) ? displaySceneUnion : null;
      if (baseAoiBbox && sceneUnion) {
        return unionBbox([baseAoiBbox, sceneUnion]) ?? baseAoiBbox;
      }
      if (baseAoiBbox) return baseAoiBbox;
      if (sceneUnion) return sceneUnion;
      return DEFAULT_BBOX;
    }

    if (baseAoiBbox) return baseAoiBbox;
    // 当 strictWorkspaceAoi 为真时，范围由 AOI 严格主导，严禁回退到场景脚印导致切源视图跳动
    if (strictWorkspaceAoi) return DEFAULT_BBOX;
    if (isLocalDisplayBbox(sceneBbox)) return sceneBbox;
    if (isLocalDisplayBbox(displaySceneUnion)) return displaySceneUnion;
    return DEFAULT_BBOX;
  }, [displayAoiFocusBbox, displaySceneUnion, fitScope, sceneBbox, strictWorkspaceAoi]);
  const fitBbox = useMemo(
    () => projectBboxForMapProvider(displayFitBbox, providerCrs),
    [displayFitBbox, providerCrs],
  );
  const [mapStatus, setMapStatus] = useState<MapRuntimeStatus>("map_initializing");
  const [layersOpen, setLayersOpen] = useState(false);
  const [localGeodataRasterOverlay, setLocalGeodataRasterOverlay] =
    useState<GeodataRasterOverlaySpec | null>(null);
  const [geodataOverlayVisible, setGeodataOverlayVisible] = useState<boolean>(true);
  useEffect(() => {
    if (geodataRasterOverlay) {
      setLocalGeodataRasterOverlay(null);
      setGeodataOverlayVisible(true);
    }
  }, [geodataRasterOverlay]);
  const effectiveGeodataOverlay = localGeodataRasterOverlay ?? geodataRasterOverlay ?? null;
  const geeLayerSnap = useSyncExternalStore(
    subscribeGeeLayerStore,
    getGeeLayerStoreSnapshot,
    getGeeLayerStoreSnapshot,
  );
  const hasVisibleGeeRasterOverlay = geeLayerSnap.layers.some((layer) => layer.visible);
  const renderedGeodataOverlayBbox = useMemo(() => {
    const rawBbox = effectiveGeodataOverlay?.bbox ?? null;
    if (!isValidBbox(rawBbox)) return null;
    const displayBbox = aoiEnvelope ? shiftBboxToDisplayDomain(rawBbox, aoiEnvelope) : rawBbox;
    return isValidBbox(displayBbox) ? projectBboxForMapProvider(displayBbox, providerCrs) : null;
  }, [aoiEnvelope, effectiveGeodataOverlay?.bbox, providerCrs]);
  const hasGeodataOverlayAvailable = Boolean(
    effectiveGeodataOverlay &&
      (effectiveGeodataOverlay.tileUrlTemplate ||
        (effectiveGeodataOverlay.previewDataUrl && isValidBbox(renderedGeodataOverlayBbox))),
  );
  const geodataOverlayToggleChip = hasGeodataOverlayAvailable ? (
    <button
      type="button"
      data-testid="btn-toggle-geodata-raster-overlay"
      aria-pressed={geodataOverlayVisible}
      onClick={(e) => {
        e.stopPropagation();
        setGeodataOverlayVisible((prev) => !prev);
      }}
      title={
        t(geodataOverlayVisible ? "panels.map.hideRasterOverlayTitle" : "panels.map.showRasterOverlayTitle", {
          label: effectiveGeodataOverlay?.label || t("panels.map.rasterLayerDefault"),
        })
      }
      className={cn(
        "pointer-events-auto h-7 px-2.5 rounded-full text-xs font-medium inline-flex items-center gap-1.5 shadow-overlay transition-colors cursor-pointer shrink-0",
        geodataOverlayVisible
          ? "bg-sp-accent-fill text-white"
          : "bg-sp-surface/90 text-sp-text-secondary hover:text-sp-text",
      )}
    >
      {geodataOverlayVisible ? <Eye className="h-3.5 w-3.5" /> : <EyeOff className="h-3.5 w-3.5" />}
      <span>{t(geodataOverlayVisible ? "panels.map.rasterLayerOn" : "panels.map.rasterLayerOff")}</span>
    </button>
  ) : null;
  const [mapApi, setMapApi] = useState<LeafletMap | null>(null);
  const [sceneTooltip, setSceneTooltip] = useState<SceneTooltipState | null>(null);
  const [mapHoveredSceneId, setMapHoveredSceneId] = useState<string | null>(null);
  const effectiveHoveredSceneId = mapHoveredSceneId ?? hoveredSceneId ?? null;
  const [aoiTooltip, setAoiTooltip] = useState<AoiTooltipState | null>(null);
  const sceneTooltipFrameRef = useRef<number | null>(null);
  const pendingSceneTooltipRef = useRef<SceneTooltipState | null>(null);
  const aoiTooltipFrameRef = useRef<number | null>(null);
  const pendingAoiTooltipRef = useRef<AoiTooltipState | null>(null);
  const scheduleSceneTooltip = useCallback((next: SceneTooltipState) => {
    pendingSceneTooltipRef.current = next;
    if (sceneTooltipFrameRef.current !== null) return;
    sceneTooltipFrameRef.current = window.requestAnimationFrame(() => {
      sceneTooltipFrameRef.current = null;
      const pending = pendingSceneTooltipRef.current;
      pendingSceneTooltipRef.current = null;
      if (pending) setSceneTooltip(pending);
    });
  }, []);
  const scheduleAoiTooltip = useCallback((next: AoiTooltipState) => {
    pendingAoiTooltipRef.current = next;
    if (aoiTooltipFrameRef.current !== null) return;
    aoiTooltipFrameRef.current = window.requestAnimationFrame(() => {
      aoiTooltipFrameRef.current = null;
      const pending = pendingAoiTooltipRef.current;
      pendingAoiTooltipRef.current = null;
      if (pending) setAoiTooltip(pending);
    });
  }, []);
  const hideSceneTooltip = useCallback(() => {
    pendingSceneTooltipRef.current = null;
    if (sceneTooltipFrameRef.current !== null) {
      window.cancelAnimationFrame(sceneTooltipFrameRef.current);
      sceneTooltipFrameRef.current = null;
    }
    setSceneTooltip(null);
  }, []);
  const hideAoiTooltip = useCallback(() => {
    pendingAoiTooltipRef.current = null;
    if (aoiTooltipFrameRef.current !== null) {
      window.cancelAnimationFrame(aoiTooltipFrameRef.current);
      aoiTooltipFrameRef.current = null;
    }
    setAoiTooltip(null);
  }, []);
  useEffect(() => () => {
    if (sceneTooltipFrameRef.current !== null) window.cancelAnimationFrame(sceneTooltipFrameRef.current);
    if (aoiTooltipFrameRef.current !== null) window.cancelAnimationFrame(aoiTooltipFrameRef.current);
  }, []);
  const onSceneSelectRef = useRef(onSceneSelect);
  useEffect(() => {
    onSceneSelectRef.current = onSceneSelect;
  }, [onSceneSelect]);

  const [activeStackScenes, setActiveStackScenes] = useState<SceneRow[] | null>(null);
  useEffect(() => {
    if (scenes.length === 0) {
      setActiveStackScenes(null);
      setMapHoveredSceneId(null);
      hideSceneTooltip();
    }
  }, [hideSceneTooltip, scenes.length]);
  const stackPopoverRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!activeStackScenes) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setActiveStackScenes(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [activeStackScenes]);

  const handleSceneSelect = useCallback(
    (sceneId: string) => {
      if (stackScenesFor) {
        const stack = stackScenesFor(sceneId);
        if (stack && stack.length >= 2) {
          setActiveStackScenes(stack);
          return;
        }
      }
      setActiveStackScenes(null);
      onSceneSelectRef.current?.(sceneId);
    },
    [stackScenesFor]
  );
  const handleSceneTooltipShow = useCallback((
    scene: SceneRow,
    index: number,
    footprint: SceneFootprintRenderModel,
    event: LeafletMouseEvent,
  ) => {
    hideAoiTooltip();
    setMapHoveredSceneId(sceneIdentity(scene));
    if (disableSceneHoverCard) {
      hideSceneTooltip();
      return;
    }
    scheduleSceneTooltip({
      scene: providerCrs === "GCJ-02"
        ? { ...scene, footprint_bbox: footprint.bbox }
        : scene,
      index,
      anchor: { x: event.containerPoint.x, y: event.containerPoint.y },
    });
  }, [disableSceneHoverCard, hideAoiTooltip, hideSceneTooltip, providerCrs, scheduleSceneTooltip]);
  const handleSceneTooltipHide = useCallback(() => {
    setMapHoveredSceneId(null);
    hideSceneTooltip();
  }, [hideSceneTooltip]);
  const handleAoiTooltipShow = useCallback((event: LeafletMouseEvent) => {
    hideSceneTooltip();
    scheduleAoiTooltip({ anchor: { x: event.containerPoint.x, y: event.containerPoint.y } });
  }, [hideSceneTooltip, scheduleAoiTooltip]);
  const handleAoiTooltipHide = hideAoiTooltip;
  const layerBackdropRef = useRef<HTMLDivElement | null>(null);
  const layerPanelRef = useRef<HTMLDivElement | null>(null);
  const visibleScenes = useMemo(() => {
    const baseScenes = admittedSceneEntries.slice(0, MAP_SCENE_DISPLAY_LIMIT);
    if (!selectedSceneId || baseScenes.some((entry) => sceneIdentity(entry.scene) === selectedSceneId)) {
      return baseScenes;
    }
    const selectedScene = admittedSceneEntries.find(
      (entry) => sceneIdentity(entry.scene) === selectedSceneId,
    );
    if (!selectedScene) return baseScenes;
    return [...baseScenes.slice(0, MAP_SCENE_DISPLAY_LIMIT - 1), selectedScene];
  }, [admittedSceneEntries, selectedSceneId]);
  const orderedSceneEntries = useMemo(() => {
    const entries = visibleScenes;
    const activeHover = effectiveHoveredSceneId ?? hoveredSceneId;
    if (!selectedSceneId && !activeHover) return entries;
    return [
      ...entries.filter(
        (entry) =>
          sceneIdentity(entry.scene) !== selectedSceneId &&
          sceneIdentity(entry.scene) !== activeHover
      ),
      ...entries.filter(
        (entry) =>
          activeHover &&
          sceneIdentity(entry.scene) === activeHover &&
          sceneIdentity(entry.scene) !== selectedSceneId
      ),
      ...entries.filter((entry) => sceneIdentity(entry.scene) === selectedSceneId),
    ];
  }, [effectiveHoveredSceneId, hoveredSceneId, selectedSceneId, visibleScenes]);
  const aoiPolygons = useMemo(() => geometryPolygons(renderedAoi), [renderedAoi]);
  const aoiOverlayBbox = aoiGeometry && !renderedAoi
    ? null
    : isValidBbox(aoiBbox)
      ? projectBboxForMapProvider(aoiBbox, providerCrs)
      : aoiBbox;
  const smokeAoiBbox = smokeMode && aoiPolygons.length === 0 && !isValidBbox(aoiBbox)
    ? projectBboxForMapProvider(UI_SMOKE_AOI_BBOX, providerCrs)
    : null;
  const aoiOverlayRendered = aoiPolygons.length > 0 || isValidBbox(aoiOverlayBbox) || smokeAoiBbox !== null;
  const aoiKmlPolygons = useMemo(() => geometryToKmlPolygons(aoiGeometry), [aoiGeometry]);
  const aoiVertexCount = useMemo(() => countGeometryVertices(aoiGeometry), [aoiGeometry]);
  const aoiAreaKm2 = useMemo(() => geometryAreaKm2(aoiGeometry), [aoiGeometry]);
  const footprintCount = visibleScenes.length;
  const hasOrbitDirection = visibleScenes.some(({ scene }) => Boolean(scene.orbit_direction?.trim()));
  const summaryBbox = mapSummaryBbox(displayFitBbox, aoiBbox);
  const bboxLabel = `W${fmt(summaryBbox.west)} S${fmt(summaryBbox.south)} E${fmt(summaryBbox.east)} N${fmt(summaryBbox.north)}`;
  const antimeridianLabel = aoiEnvelope?.crossesAntimeridian
    ? t("panels.map.crossesAntimeridian", { windows: aoiEnvelope.requestWindows.length })
    : null;
  const copyLabel =
    antimeridianLabel
    ?? (aoiKmlPolygons.length > 0 && aoiVertexCount > 0
      ? aoiKmlPolygons.length > 1
        ? t("panels.map.aoiCopyLabelMulti", { polygons: aoiKmlPolygons.length, vertices: aoiVertexCount, bbox: bboxLabel })
        : t("panels.map.aoiCopyLabel", { vertices: aoiVertexCount, bbox: bboxLabel })
      : bboxLabel);
  const kmlCopyText = useMemo(() => {
    const polygons = aoiKmlPolygons.length > 0 ? aoiKmlPolygons : [bboxPolygonKml(summaryBbox)];
    return kmlDocument(aoiKmlPolygons.length > 0 ? "EOStage AOI" : "EOStage AOI bbox", summaryBbox, polygons);
  }, [aoiKmlPolygons, summaryBbox]);

  const hasAuthoritativeAoi = isValidBbox(aoiBbox);
  const displaySummaryLabel = strictWorkspaceAoi && !hasAuthoritativeAoi
    ? t("panels.map.globalViewUnrestricted")
    : copyLabel;
  const isKmlCopyDisabled = strictWorkspaceAoi && !hasAuthoritativeAoi;
  const effectiveKmlCopyText = isKmlCopyDisabled ? "" : kmlCopyText;
  const declutterLegendItems = useMemo(() => {
    if (!declutter) return [];
    const items: { key: string; dot: ReactNode; label: string }[] = [];

    if (aoiOverlayRendered) {
      items.push({
        key: "aoi",
        dot: <span className="w-2 h-2 rounded-full bg-[#eab308] ring-1 ring-amber-700/60 shrink-0" />,
        label: t("panels.map.legendAoiBoundary"),
      });
    }

    if (renderedDemEnvelopeBbox) {
      items.push({
        key: "dem",
        dot: (
          <span className="w-2 h-2 rounded-full bg-amber-500 ring-1 ring-amber-700/60 shrink-0" />
        ),
        label: t("panels.map.legendDemExtent"),
      });
    }

    if (footprintCount > 0) {
      let hasAsc = false;
      let hasDesc = false;
      let hasUnknown = false;
      let hasOpt = false;

      for (const { scene } of visibleScenes) {
        if (isOpticalScene(scene)) {
          hasOpt = true;
        } else {
          const dir = (scene.orbit_direction ?? "").toUpperCase();
          if (dir === "ASCENDING" || dir === "A") {
            hasAsc = true;
          } else if (dir === "DESCENDING" || dir === "D") {
            hasDesc = true;
          } else {
            hasUnknown = true;
          }
        }
      }

      if (hasAsc) {
        items.push({
          key: "asc",
          dot: (
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ background: MAP_ASC_COLOR }}
            />
          ),
          label: orbitLabels?.ascending ?? t("panels.map.ascending"),
        });
      }
      if (hasDesc) {
        items.push({
          key: "desc",
          dot: (
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ background: MAP_DESC_COLOR }}
            />
          ),
          label: orbitLabels?.descending ?? t("panels.map.descending"),
        });
      }
      if (hasUnknown && hasOrbitDirection) {
        items.push({
          key: "unknown",
          dot: <span className="w-2 h-2 rounded-full bg-[#64748b] shrink-0" />,
          label: orbitLabels?.unknown ?? t("panels.map.unknown"),
        });
      }
      if (hasOpt) {
        items.push({
          key: "optical",
          dot: <span className="w-2 h-2 rounded-full shrink-0" style={{ background: "#8b5cf6" }} />,
          label: t("panels.map.optical"),
        });
      }
    }

    return items;
  }, [
    declutter,
    aoiOverlayRendered,
    renderedDemEnvelopeBbox,
    footprintCount,
    visibleScenes,
    hasOrbitDirection,
    orbitLabels,
    t,
  ]);

  useEffect(() => {
    if (declutterLegendItems.length === 0) return;
    const measureDock = () => {
      const leftEl = legendTriggerLeftRef.current;
      const gridEl = legendGridContentRef.current;
      const leftW = leftEl && leftEl.scrollWidth > 0
        ? Math.ceil(leftEl.scrollWidth)
        : 44 + declutterLegendItems.length * 12;
      const closedW = Math.max(88, leftW + 38);
      const gridW = gridEl && gridEl.scrollWidth > 0 ? Math.ceil(gridEl.scrollWidth) + 2 : 176;
      const gridH = gridEl && gridEl.scrollHeight > 0 ? Math.ceil(gridEl.scrollHeight) : 52;
      const openW = Math.max(closedW + 12, gridW);
      const openH = 28 + gridH + 2;
      setDockMetrics((prev) =>
        prev.closedW === closedW && prev.openW === openW && prev.openH === openH
          ? prev
          : { closedW, openW, openH }
      );
    };
    measureDock();
    let rafId: number | null = null;
    if (typeof window !== "undefined" && typeof window.requestAnimationFrame === "function") {
      rafId = window.requestAnimationFrame(measureDock);
    }
    return () => {
      if (rafId !== null && typeof window !== "undefined" && typeof window.cancelAnimationFrame === "function") {
        window.cancelAnimationFrame(rafId);
      }
    };
  }, [declutterLegendItems, legendOpen]);

  const [bboxCopied, setBboxCopied] = useState(false);
  const copyFitBbox = useCallback(() => {
    const done = () => {
      setBboxCopied(true);
      window.setTimeout(() => setBboxCopied(false), 1200);
    };
    if (isKmlCopyDisabled) return;
    const writer = navigator.clipboard?.writeText?.(effectiveKmlCopyText);
    if (writer) {
      void writer.then(done).catch(done);
    } else {
      done();
    }
  }, [kmlCopyText]);
  const previewAuthorityState = previewRenderAuthority === undefined
    ? aoiDisplayAuthority ? "mismatch" : "legacy"
    : aoiDisplayAuthority &&
      previewRenderAuthority.region_id === aoiDisplayAuthority.region_id &&
      previewRenderAuthority.revision === aoiDisplayAuthority.revision
      ? "native"
      : "mismatch";
  const nativePreviewRenderAuthority = previewAuthorityState === "native" && previewRenderAuthority !== undefined
    ? previewRenderAuthority
    : null;
  const strictPreviewAuthorityMismatch = previewAuthorityState === "mismatch"
    && Boolean(aoiDisplayAuthority?.envelope.crossesAntimeridian);
  const displayPreviewGeometry = nativePreviewRenderAuthority
    ? nativePreviewRenderAuthority.render_geometry
    : strictPreviewAuthorityMismatch
      ? null
      : previewRenderGeometry
        ?? (aoiEnvelope && previewGeometry
          ? isFullWorldDisplayEnvelope(aoiEnvelope)
            ? previewGeometry
            : shiftPolygonalGeojsonToDisplayDomain(previewGeometry, aoiEnvelope)
          : previewGeometry);
  const displayPreviewBbox = nativePreviewRenderAuthority
    ? nativePreviewRenderAuthority.render_bbox
    : strictPreviewAuthorityMismatch
      ? null
      : aoiEnvelope
        ? (
            displayPreviewGeometry
              ? sceneFootprintRenderModel(previewGeometry, displayPreviewGeometry)?.bbox
                ?? (previewRenderGeometry ? previewRenderBbox ?? previewBbox : null)
              : shiftBboxToDisplayDomain(previewRenderBbox ?? previewBbox, aoiEnvelope)
          )
        : previewRenderBbox ?? previewBbox;
  const renderedPreviewGeometry = displayPreviewGeometry
    ? projectGeometryForMapProvider(displayPreviewGeometry, providerCrs)
    : null;
  const renderedPreviewBbox = isValidBbox(displayPreviewBbox)
    ? projectBboxForMapProvider(displayPreviewBbox, providerCrs)
    : displayPreviewBbox;
  const previewTileLayerVisible = Boolean(
    previewTileLayer
    && preview
    && ["loading", "available", "failed"].includes(preview.status)
    && preview.product_key === previewTileLayer.productKey
    && activePreviewProductKey === previewTileLayer.productKey,
  );
  const [previewTileRuntime, setPreviewTileRuntime] = useState<{
    identity: string;
    status: "loading" | "ready" | "failed";
    loadedCount: number;
    errorCount: number;
  } | null>(null);
  useEffect(() => {
    setPreviewTileRuntime(
      previewTileLayerVisible && previewTileLayer
        ? { identity: previewTileLayer.identity, status: "loading", loadedCount: 0, errorCount: 0 }
        : null,
    );
  }, [previewTileLayer?.identity, previewTileLayerVisible]);
  useEffect(() => {
    if (
      preview?.status !== "loading"
      || !previewTileLayerVisible
      || !previewTileLayer
    ) return;
    setPreviewTileRuntime((current) => (
      current?.identity === previewTileLayer.identity && current.status === "loading"
        ? current
        : { identity: previewTileLayer.identity, status: "loading", loadedCount: 0, errorCount: 0 }
    ));
  }, [preview?.status, previewTileLayer?.identity, previewTileLayerVisible]);
  const previewTileStatus = previewTileLayerVisible && previewTileLayer
    ? previewTileRuntime?.identity === previewTileLayer.identity
      ? previewTileRuntime.status
      : "loading"
    : null;
  useEffect(() => {
    if (
      !onPreviewTileStatusChange
      || !previewTileLayerVisible
      || !previewTileLayer
      || previewTileRuntime?.identity !== previewTileLayer.identity
    ) return;
    onPreviewTileStatusChange({
      productKey: previewTileLayer.productKey,
      identity: previewTileLayer.identity,
      status: previewTileRuntime.status,
      loadedCount: previewTileRuntime.loadedCount,
      errorCount: previewTileRuntime.errorCount,
    });
  }, [
    onPreviewTileStatusChange,
    previewTileLayer?.identity,
    previewTileLayer?.productKey,
    previewTileLayerVisible,
    previewTileRuntime?.identity,
    previewTileRuntime?.status,
    previewTileRuntime?.loadedCount,
    previewTileRuntime?.errorCount,
  ]);
  const previewMapVisible = Boolean(
    !previewTileLayerVisible
    && !unsupportedAoiProjection
    && !strictPreviewAuthorityMismatch
    && (previewAuthorityState !== "native" || renderedPreviewGeometry !== null)
    && (previewAuthorityState !== "native" ||
      renderedPreviewBbox !== null && renderedPreviewBbox.east - renderedPreviewBbox.west <= 180)
    &&
    preview?.status === "available"
    && preview.url
    && preview.product_key
    && preview.product_key === activePreviewProductKey
    && (!preview.descriptor?.render_profile_id || preview.descriptor.render_profile_id === ASF_SENTINEL1_GRD_GEOGRAPHIC_PREVIEW_PROFILE)
    && isValidBbox(renderedPreviewBbox),
  );
  const displaySceneBbox = unsupportedAoiProjection
    ? null
    : aoiEnvelope
    ? shiftBboxToDisplayDomain(sceneBbox, aoiEnvelope)
    : sceneBbox;
  const renderedSceneBbox = isValidBbox(displaySceneBbox)
    ? projectBboxForMapProvider(displaySceneBbox, providerCrs)
    : displaySceneBbox;

  const [baseTileRuntime, setBaseTileRuntime] = useState<BaseTileRuntime>(() =>
    initialBaseTileRuntime(),
  );
  const isCartoLayer = effectiveLayerKey === "cartoDark" || effectiveLayerKey === "cartoLight";
  // CARTO requires an API key since August 2026 and returns HTTP 200 "API KEY REQUIRED"
  // watermark tiles for keyless requests; route keyless Carto layers directly to Esri Gray Canvas.
  const [cartoFallbackActive, setCartoFallbackActive] = useState(isCartoLayer);

  useEffect(() => {
    setCartoFallbackActive(isCartoLayer);
    setBaseTileRuntime(initialBaseTileRuntime());
  }, [effectiveLayerKey, isCartoLayer]);
  const effectiveBaseTileUrl = isCartoLayer && cartoFallbackActive
    ? effectiveLayerKey === "cartoDark"
      ? "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Base/MapServer/tile/{z}/{y}/{x}"
      : "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Base/MapServer/tile/{z}/{y}/{x}"
    : tileUrlWithUserToken(layer.url, effectiveToken);
  const effectiveBaseSubdomains = isCartoLayer && cartoFallbackActive ? [] : layer.subdomains;
  const effectiveBaseAttribution =
    isCartoLayer && cartoFallbackActive ? "&copy; Esri" : layer.attribution;
  const effectiveBaseAttributionFull =
    isCartoLayer && cartoFallbackActive
      ? "Esri, HERE, Garmin, © OpenStreetMap contributors"
      : layer.attributionFull || effectiveBaseAttribution;

  const attributionLayerCredits = useMemo(() => {
    if (!declutter) return [];
    const credits: string[] = [];
    const clean = (text: string) =>
      text
        .replaceAll("&copy;", "©")
        .replaceAll("&amp;", "&")
        .replaceAll("天地图", t("panels.mapAttribution.tianditu"))
        .replaceAll("高德地图", t("panels.mapAttribution.amap"))
        .trim();

    if (effectiveBaseAttribution) {
      const cleaned = clean(effectiveBaseAttribution);
      if (cleaned && !credits.includes(cleaned)) credits.push(cleaned);
    }
    for (const annotation of MAP_ANNOTATION_PROVIDERS) {
      const enabled = annotation.z_index_role === "imageAnnotation"
        ? effectiveAnnotations.image
        : effectiveAnnotations.vector;
      if (enabled && hasTiandituToken && annotation.attribution) {
        const cleaned = clean(annotation.attribution);
        if (cleaned && !credits.includes(cleaned)) credits.push(cleaned);
      }
    }
    if (
      previewTileLayerVisible &&
      previewTileLayer &&
      previewTileStatus !== "failed" &&
      isValidBbox(renderedPreviewBbox) &&
      previewTileLayer.attribution
    ) {
      const cleaned = clean(previewTileLayer.attribution);
      if (cleaned && !credits.includes(cleaned)) credits.push(cleaned);
    }
    return credits;
  }, [
    declutter,
    effectiveBaseAttribution,
    effectiveAnnotations,
    hasTiandituToken,
    previewTileLayerVisible,
    previewTileLayer,
    previewTileStatus,
    renderedPreviewBbox,
    locale,
    t,
  ]);

  const attributionCreditsText = useMemo(() => {
    if (!declutter) return "";
    const fullBase = effectiveBaseAttributionFull
      .replaceAll("&copy;", "©")
      .replaceAll("&amp;", "&")
      .replaceAll("天地图", t("panels.mapAttribution.tianditu"))
      .replaceAll("高德地图", t("panels.mapAttribution.amap"))
      .trim();
    const extraCredits = attributionLayerCredits.slice(1);
    const parts = ["Leaflet", ...(fullBase ? [fullBase] : []), ...extraCredits];
    return parts.join(" · ");
  }, [declutter, effectiveBaseAttributionFull, attributionLayerCredits, t]);

  useEffect(() => {
    if (baseTileRuntime.status === "loading") {
      setMapStatus("external_tiles_loading");
    }
  }, [baseTileRuntime.status, effectiveLayerKey]);

  useEffect(() => {
    const onSecurityPolicyViolation = (event: SecurityPolicyViolationEvent) => {
      if (baseTileRuntime.loadedCount > 0) {
        return;
      }
      if (event.effectiveDirective === "img-src" || event.violatedDirective === "img-src") {
        const blockedUri = event.blockedURI || "";
        let matchesBaseLayer = false;
        try {
          const tileTemplate = layer?.url?.replace("{s}", "a") || "";
          const tileHost = tileTemplate ? new URL(tileTemplate).hostname : "";
          const blockedHost = blockedUri ? new URL(blockedUri).hostname : "";
          matchesBaseLayer = Boolean(tileHost && blockedHost && tileHost === blockedHost);
        } catch {
          matchesBaseLayer = false;
        }
        if (matchesBaseLayer) {
          setMapStatus("csp_or_network_blocked");
        }
      }
    };
    window.addEventListener("securitypolicyviolation", onSecurityPolicyViolation);
    return () => window.removeEventListener("securitypolicyviolation", onSecurityPolicyViolation);
  }, [effectiveLayerKey, baseTileRuntime.loadedCount, layer?.url]);

  useEffect(() => {
    if (isCartoLayer && !cartoFallbackActive) {
      return;
    }
    if (baseTileRuntime.status !== "loading" || baseTileRuntime.errorCount > 0) {
      return;
    }
    const timer = window.setTimeout(() => {
      setBaseTileRuntime((rt) =>
        rt.status === "loading" && rt.errorCount === 0
          ? { ...rt, status: "ready" }
          : rt,
      );
    }, 2800);
    return () => window.clearTimeout(timer);
  }, [effectiveLayerKey, isCartoLayer, cartoFallbackActive, baseTileRuntime.status, baseTileRuntime.errorCount]);

  useEffect(() => {
    if (!isCartoLayer || cartoFallbackActive || baseTileRuntime.loadedCount > 0) {
      return;
    }
    const timer = window.setTimeout(() => {
      setCartoFallbackActive(true);
      setBaseTileRuntime(initialBaseTileRuntime());
    }, 2500);
    return () => window.clearTimeout(timer);
  }, [effectiveLayerKey, isCartoLayer, cartoFallbackActive, baseTileRuntime.loadedCount]);

  useEffect(() => {
    if (baseTileRuntime.status === "ready" || baseTileRuntime.loadedCount > 0) {
      setMapStatus("external_tiles_ready");
    } else if (baseTileRuntime.status === "failed") {
      setMapStatus("external_tiles_failed");
    }
  }, [baseTileRuntime.status, baseTileRuntime.loadedCount]);

  useEffect(() => {
    if (!layersOpen) return;
    for (const element of [layerBackdropRef.current, layerPanelRef.current]) {
      if (!element) continue;
      DomEvent.disableClickPropagation(element);
      DomEvent.disableScrollPropagation(element);
    }
  }, [layersOpen]);

  return (
    <div
      ref={containerRef}
      data-base-layer={effectiveLayerKey}
      data-carto-fallback={isCartoLayer && cartoFallbackActive ? "true" : "false"}
      className="relative h-full w-full overflow-hidden bg-[#d9e6e2]"
      onDragEnter={(e) => {
        if (!onVectorAoiImported) return;
        const hasFiles = Array.from(e.dataTransfer?.types ?? []).includes("Files");
        if (!hasFiles && (!e.dataTransfer?.files || e.dataTransfer.files.length === 0)) return;
        vectorDragDepthRef.current += 1;
        e.preventDefault();
        setHtmlVectorDrag(true);
      }}
      onDragOver={(e) => {
        if (!onVectorAoiImported) return;
        const hasFiles = Array.from(e.dataTransfer?.types ?? []).includes("Files");
        if (!hasFiles && (!e.dataTransfer?.files || e.dataTransfer.files.length === 0)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }}
      onDragLeave={() => {
        if (!onVectorAoiImported) return;
        vectorDragDepthRef.current = Math.max(0, vectorDragDepthRef.current - 1);
        if (vectorDragDepthRef.current === 0) {
          setHtmlVectorDrag(false);
        }
      }}
      onDrop={(e) => {
        if (!onVectorAoiImported) return;
        vectorDragDepthRef.current = 0;
        setHtmlVectorDrag(false);
        const files = e.dataTransfer?.files;
        if (!files || files.length === 0) return;
        e.preventDefault();
        e.stopPropagation();
        setVectorImportBusy(true);
        void importVectorFromHtml5Files(files, { persistWorkingAoi: true })
          .then(applyDroppedVectorResult)
          .finally(() => setVectorImportBusy(false));
      }}
    >
      <MapContainer
        center={centerFromBbox(fitBbox)}
        zoom={10}
        scrollWheelZoom
        zoomControl={false}
        attributionControl={false}
        className={cn("relative z-0 isolate h-full w-full", drawActive && "cursor-crosshair")}
      >
        {!declutter && <AttributionControl />}
        <TileLayer
          key={`base:${effectiveLayerKey}:${cartoFallbackActive ? "fallback" : "primary"}`}
          zIndex={MAP_TILE_Z_INDEX.base}
          url={effectiveBaseTileUrl}
          subdomains={effectiveBaseSubdomains}
          attribution={effectiveBaseAttribution}
          maxNativeZoom={isCartoLayer && cartoFallbackActive ? 16 : layer.maxZoom}
          bounds={[
            [-85.05112878, -540],
            [85.05112878, 540],
          ]}
          updateWhenIdle
          updateWhenZooming={false}
          keepBuffer={2}
          detectRetina={false}
          eventHandlers={{
            load: () => setBaseTileRuntime((rt) => baseTileOnLoadEvent(rt)),
            tileload: () => setBaseTileRuntime((rt) => baseTileOnTileload(rt)),
            tileerror: () => {
              if (isCartoLayer && !cartoFallbackActive) {
                setBaseTileRuntime((rt) => {
                  if (rt.loadedCount === 0) {
                    setCartoFallbackActive(true);
                    return initialBaseTileRuntime();
                  }
                  return baseTileOnTileerror(rt);
                });
                return;
              }
              setBaseTileRuntime((rt) => baseTileOnTileerror(rt));
            },
          }}
        />
        {isCartoLayer && cartoFallbackActive && (
          <TileLayer
            key={`base-ref:${effectiveLayerKey}`}
            zIndex={MAP_TILE_Z_INDEX.base + 1}
            url={
              effectiveLayerKey === "cartoDark"
                ? "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Dark_Gray_Reference/MapServer/tile/{z}/{y}/{x}"
                : "https://server.arcgisonline.com/ArcGIS/rest/services/Canvas/World_Light_Gray_Reference/MapServer/tile/{z}/{y}/{x}"
            }
            maxNativeZoom={16}
            bounds={[
              [-85.05112878, -540],
              [85.05112878, 540],
            ]}
            updateWhenIdle
            updateWhenZooming={false}
            keepBuffer={2}
            detectRetina={false}
          />
        )}
        {pinnedPreviews !== undefined ? (
          !unsupportedAoiProjection &&
          pinnedPreviews.length > 0 && (
            <Pane
              name="catalog-preview-image-pane"
              style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX, pointerEvents: "none" }}
            >
              {pinnedPreviews.map((entry) => {
                const displayEntryBbox = aoiEnvelope
                  ? shiftBboxToDisplayDomain(entry.bbox, aoiEnvelope)
                  : entry.bbox;
                const projectedEntryBbox = isValidBbox(displayEntryBbox)
                  ? projectBboxForMapProvider(displayEntryBbox, providerCrs)
                  : null;
                if (!isValidBbox(projectedEntryBbox)) return null;
                if (entry.tileLayer) {
                  const isCoarseOptical = entry.tileLayer.urlTemplate.includes("landsat") || entry.tileLayer.urlTemplate.includes("hls");
                  return (
                    <TileLayer
                      key={`pinned-tile:${entry.identity}`}
                      pane="catalog-preview-image-pane"
                      url={entry.tileLayer.urlTemplate}
                      bounds={boundsFromBbox(projectedEntryBbox)}
                      attribution={entry.tileLayer.attribution}
                      maxNativeZoom={isCoarseOptical ? 13 : 15}
                      maxZoom={20}
                      opacity={1}
                      updateWhenIdle
                      updateWhenZooming={false}
                      keepBuffer={2}
                      detectRetina={false}
                      referrerPolicy="no-referrer"
                    />
                  );
                }
                if (!entry.url) return null;
                const projectedEntryGeom = entry.footprintGeojson
                  ? projectGeometryForMapProvider(
                      (aoiEnvelope && !isFullWorldDisplayEnvelope(aoiEnvelope)
                        ? shiftPolygonalGeojsonToLongitudeReference(entry.footprintGeojson as Json, aoiEnvelope) ??
                          (entry.footprintGeojson as Json)
                        : (entry.footprintGeojson as Json)) as any,
                      providerCrs,
                    )
                  : null;
                return (
                  <PreviewImageOverlay
                    key={`pinned-img:${entry.identity}`}
                    identity={entry.identity}
                    url={entry.url}
                    bounds={boundsFromBbox(projectedEntryBbox)}
                    footprintGeojson={projectedEntryGeom}
                    renderImage={({ url, bounds, opacity, className, clipPath, onLoad, onError }) => (
                      <ImageOverlay
                        pane="catalog-preview-image-pane"
                        url={url}
                        bounds={bounds}
                        opacity={opacity}
                        className={className}
                        clipPath={clipPath}
                        zIndex={MAP_TILE_Z_INDEX.preview}
                        interactive={false}
                        eventHandlers={{
                          add: (ev) => applyPreviewPolygonClip(ev.target, clipPath),
                          load: (ev) => {
                            applyPreviewPolygonClip(ev.target, clipPath);
                            onLoad();
                          },
                          error: () => {
                            onError();
                            onPinnedPreviewError?.(entry.sceneId);
                          },
                        }}
                      />
                    )}
                  />
                );
              })}
            </Pane>
          )
        ) : (
          <>
            {previewTileLayerVisible && previewTileLayer && previewTileStatus !== "failed" && isValidBbox(renderedPreviewBbox) && (
              <Pane
                name="catalog-preview-tile-pane"
                style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX, pointerEvents: "none" }}
              >
                <TileLayer
                  key={`catalog-preview:${previewTileLayer.identity}`}
                  url={previewTileLayer.urlTemplate}
                  bounds={boundsFromBbox(renderedPreviewBbox!)}
                  attribution={previewTileLayer.attribution}
                  maxNativeZoom={
                    previewTileLayer.urlTemplate.includes("landsat") || previewTileLayer.urlTemplate.includes("hls")
                      ? 13
                      : 15
                  }
                  maxZoom={20}
                  opacity={1}
                  updateWhenIdle
                  updateWhenZooming={false}
                  keepBuffer={2}
                  detectRetina={false}
                  referrerPolicy="no-referrer"
                  eventHandlers={{
                    load: () => setPreviewTileRuntime((current) => current?.identity === previewTileLayer.identity
                      ? current.loadedCount > 0
                        ? { ...current, status: "ready" }
                        : current.errorCount > 0
                          ? { ...current, status: "failed" }
                          : current
                      : current),
                    tileload: () => setPreviewTileRuntime((current) => current?.identity === previewTileLayer.identity
                      ? {
                          ...current,
                          status: "ready",
                          loadedCount: current.loadedCount + 1,
                        }
                      : current),
                    tileerror: () => setPreviewTileRuntime((current) => {
                      if (current?.identity !== previewTileLayer.identity) return current;
                      if (current.status === "ready" || current.loadedCount > 0) return current;
                      const errorCount = current.errorCount + 1;
                      return {
                        ...current,
                        errorCount,
                        status: errorCount >= 4 ? "failed" : "loading",
                      };
                    }),
                  }}
                />
              </Pane>
            )}
            {previewMapVisible && preview?.url && isValidBbox(renderedPreviewBbox) && (
              <Pane
                name="catalog-preview-image-pane"
                style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX, pointerEvents: "none" }}
              >
                <PreviewImageOverlay
                  identity={`${preview.product_key}:${preview.url}`}
                  url={preview.url}
                  bounds={boundsFromBbox(renderedPreviewBbox)}
                  footprintGeojson={renderedPreviewGeometry}
                  renderImage={({ url, bounds, opacity, className, clipPath, onLoad, onError }) => (
                    <ImageOverlay
                      pane="catalog-preview-image-pane"
                      url={url}
                      bounds={bounds}
                      opacity={opacity}
                      className={className}
                      clipPath={clipPath}
                      zIndex={MAP_TILE_Z_INDEX.preview}
                      interactive={false}
                      eventHandlers={{
                        add: (ev) => applyPreviewPolygonClip(ev.target, clipPath),
                        load: (ev) => {
                          applyPreviewPolygonClip(ev.target, clipPath);
                          onLoad();
                        },
                        error: onError,
                      }}
                    />
                  )}
                />
              </Pane>
            )}
          </>
        )}
        {geodataOverlayVisible && hasGeodataOverlayAvailable && effectiveGeodataOverlay && (
          <Pane
            name="geodata-raster-overlay-pane"
            style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX + 1, pointerEvents: "none" }}
          >
            {effectiveGeodataOverlay.tileUrlTemplate && (
              <TileLayer
                key={`geodata-tile:${effectiveGeodataOverlay.id}`}
                pane="geodata-raster-overlay-pane"
                zIndex={MAP_TILE_Z_INDEX.preview + 1}
                url={effectiveGeodataOverlay.tileUrlTemplate}
                attribution={effectiveGeodataOverlay.attribution}
                bounds={
                  isValidBbox(renderedGeodataOverlayBbox)
                    ? boundsFromBbox(renderedGeodataOverlayBbox)
                    : undefined
                }
                maxNativeZoom={effectiveGeodataOverlay.maxZoom ?? 20}
                opacity={0.88}
                updateWhenIdle
                updateWhenZooming={false}
                keepBuffer={0}
                detectRetina={false}
              />
            )}
            {effectiveGeodataOverlay.previewDataUrl &&
              isValidBbox(renderedGeodataOverlayBbox) && (
                <ImageOverlay
                  key={`geodata-img:${effectiveGeodataOverlay.id}`}
                  pane="geodata-raster-overlay-pane"
                  url={effectiveGeodataOverlay.previewDataUrl}
                  bounds={boundsFromBbox(renderedGeodataOverlayBbox)}
                  opacity={0.88}
                  zIndex={MAP_TILE_Z_INDEX.preview + 2}
                  interactive={false}
                  className="geodata-raster-image-overlay"
                />
              )}
          </Pane>
        )}
        {MAP_ANNOTATION_PROVIDERS.map((annotation) => {
          const enabled = annotation.z_index_role === "imageAnnotation"
            ? effectiveAnnotations.image
            : effectiveAnnotations.vector;
          if (!enabled || !hasTiandituToken) return null;
          return (
          <TileLayer
            key={`annotation:${annotation.key}`}
            zIndex={MAP_TILE_Z_INDEX[annotation.z_index_role]}
            url={tileUrlWithUserToken(annotation.tile_url_template, effectiveToken)}
            subdomains={annotation.subdomains}
            attribution={annotation.attribution}
            updateWhenIdle
            updateWhenZooming={false}
            keepBuffer={2}
            detectRetina={false}
          />
          );
        })}
        <MapApiBridge onReady={setMapApi} />
        <ResizeMapWithContainer />
        <FitToData
          bbox={fitBbox}
          aoiFocusBbox={aoiFocusBbox}
          lockViewportOnSceneChange={lockViewportOnSceneChange}
          focusBoundsRequest={focusBoundsRequest}
        />
        {subsetFootprintGeometries.map((itemGeometry, index) => (
          <AoiOverlay
            key={`subset-footprint:${index}`}
            geometry={projectGeometryForMapProvider(itemGeometry, providerCrs)}
            bbox={null}
            renderPolygon={({ key, positions }) => (
              <Polygon
                key={key}
                positions={positions!}
                pathOptions={{
                  color: "#475569",
                  weight: 1.5,
                  dashArray: "5 4",
                  fillColor: "#64748b",
                  fillOpacity: 0.06,
                  className: "subset-scene-footprint",
                }}
                interactive={false}
              />
            )}
            renderRectangle={({ key, bounds }) => (
              <Rectangle
                key={key}
                bounds={bounds!}
                pathOptions={{
                  color: "#475569",
                  weight: 1.5,
                  dashArray: "5 4",
                  fillColor: "#64748b",
                  fillOpacity: 0.06,
                  className: "subset-scene-footprint",
                }}
                interactive={false}
              />
            )}
          />
        ))}
        {subsetPreviewImageUrl && isValidBbox(subsetPreviewImageBbox) && (
          <Pane
            name="subset-preview-image-pane"
            style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX, pointerEvents: "none" }}
          >
            <PreviewImageOverlay
              identity={`subset-preview:${subsetPreviewImageUrl}`}
              url={subsetPreviewImageUrl}
              bounds={boundsFromBbox(projectBboxForMapProvider(subsetPreviewImageBbox, providerCrs))}
              footprintGeojson={
                subsetPreviewGeometry
                  ? projectGeometryForMapProvider(subsetPreviewGeometry, providerCrs)
                  : null
              }
              renderImage={({ url, bounds, opacity, className, clipPath, onLoad, onError }) => (
                <ImageOverlay
                  pane="subset-preview-image-pane"
                  url={url}
                  bounds={bounds}
                  opacity={opacity}
                  className={className}
                  clipPath={clipPath}
                  zIndex={MAP_TILE_Z_INDEX.preview}
                  interactive={false}
                  eventHandlers={{
                    add: (ev) => applyPreviewPolygonClip(ev.target, clipPath),
                    load: (ev) => {
                      applyPreviewPolygonClip(ev.target, clipPath);
                      onLoad();
                    },
                    error: onError,
                  }}
                />
              )}
            />
          </Pane>
        )}
        {subsetPreviewGeometry && (
          <AoiOverlay
            geometry={projectGeometryForMapProvider(subsetPreviewGeometry, providerCrs)}
            bbox={null}
            renderPolygon={({ key, positions }) => (
              <Polygon
                key={`subset-preview:${key}`}
                positions={positions!}
                pathOptions={{
                  color: "#d97706",
                  weight: 2.2,
                  fillColor: "#f59e0b",
                  fillOpacity: 0.06,
                  className: "subset-preview-footprint",
                }}
                interactive={false}
              />
            )}
            renderRectangle={({ key, bounds }) => (
              <Rectangle
                key={`subset-preview:${key}`}
                bounds={bounds!}
                pathOptions={{
                  color: "#d97706",
                  weight: 2.2,
                  fillColor: "#f59e0b",
                  fillOpacity: 0.06,
                  className: "subset-preview-footprint",
                }}
                interactive={false}
              />
            )}
          />
        )}
        {subsetEffectiveGeometries.map((itemGeometry, index) => (
          <Pane
            key={`subset-effective-pane:${index}`}
            name={`subset-effective-pane-${index}`}
            style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX + 10, pointerEvents: "none" }}
          >
            <AoiOverlay
              geometry={projectGeometryForMapProvider(itemGeometry, providerCrs)}
              bbox={null}
              renderPolygon={({ key, positions }) => (
                <Polygon
                  key={`subset-effective:${index}:${key}`}
                  positions={positions!}
                  pathOptions={{
                    color: "#059669",
                    weight: 2.2,
                    fillColor: "#10b981",
                    fillOpacity: 0.2,
                    className: "subset-effective-intersection",
                  }}
                  interactive={false}
                />
              )}
              renderRectangle={({ key, bounds }) => (
                <Rectangle
                  key={`subset-effective:${index}:${key}`}
                  bounds={bounds!}
                  pathOptions={{
                    color: "#059669",
                    weight: 2.2,
                    fillColor: "#10b981",
                    fillOpacity: 0.2,
                    className: "subset-effective-intersection",
                  }}
                  interactive={false}
                />
              )}
            />
          </Pane>
        ))}
        {renderedCandidateBboxes.map((candidate, index) => (
          <Rectangle
            key={"admin-boundary-candidate:" + index + ":" + candidate.west + ":" + candidate.south + ":" + candidate.east + ":" + candidate.north}
            bounds={boundsFromBbox(candidate)}
            pathOptions={{
              color: "#0891b2",
              weight: 1.5,
              dashArray: "5 5",
              fillOpacity: 0.03,
              className: "admin-boundary-candidate",
            }}
            interactive={false}
          />
        ))}
        {isLocalDisplayBbox(renderedSceneBbox) && (
          <Rectangle
            bounds={boundsFromBbox(renderedSceneBbox)}
            pathOptions={{ color: "#f59e0b", weight: 2, dashArray: "8 5", fillOpacity: 0.08 }}
            interactive={false}
          />
        )}
        {renderedDemEnvelopeBbox && (
          <Rectangle
            bounds={boundsFromBbox(renderedDemEnvelopeBbox)}
            pathOptions={{
              color: "#d97706",
              weight: 3,
              dashArray: "9 6",
              fillColor: "#f59e0b",
              fillOpacity: 0.08,
              className: "dem-download-envelope",
            }}
            interactive={false}
          />
        )}
        <Pane
          name="aoi-boundary-pane"
          style={{ zIndex: MAP_PREVIEW_PANE_Z_INDEX + 15, pointerEvents: "none" }}
        >
          {(() => {
            const hasActiveRasterOverlay = Boolean(
              (previewMapVisible && preview?.url) ||
              (previewTileLayer?.urlTemplate && renderedPreviewBbox) ||
              (pinnedPreviews && pinnedPreviews.length > 0) ||
              (effectiveGeodataOverlay && renderedGeodataOverlayBbox && geodataOverlayVisible) ||
              hasVisibleGeeRasterOverlay ||
              swipeLeftLayer ||
              swipeRightLayer,
            );
            return (
              <>
                {adminPreviewGeometry && (
                  <AoiOverlay
                    geometry={adminPreviewGeometry}
                    bbox={null}
                    suppressInteriorFill={hasActiveRasterOverlay}
                    renderPolygon={({ key, positions }) => (
                      <Polygon
                        key={`admin-preview:${key}`}
                        positions={positions!}
                        pathOptions={{ color: "#2563eb", weight: 2, dashArray: "6 5", fill: false, className: "admin-boundary-preview" }}
                        interactive={false}
                      />
                    )}
                    renderRectangle={() => null}
                  />
                )}
                <AoiOverlay
                  geometry={aoiGeometry}
                  renderGeometry={renderedAoi}
                  bbox={aoiOverlayBbox}
                  suppressInteriorFill={hasActiveRasterOverlay}
                  renderPolygon={({ key, positions, pathOptions }) => (
                    <Fragment key={key}>
                      <Polygon
                        positions={positions!}
                        pathOptions={{
                          color: "#ffffff",
                          weight: 5.5,
                          opacity: 0.85,
                          fill: false,
                          className: "aoi-overlay-halo",
                        }}
                        interactive={false}
                      />
                      <Polygon positions={positions!} pathOptions={pathOptions} interactive={false} />
                    </Fragment>
                  )}
                  renderRectangle={({ key, bounds, pathOptions }) => (
                    <Fragment key={key}>
                      <Rectangle
                        bounds={bounds!}
                        pathOptions={{
                          color: "#ffffff",
                          weight: 5,
                          opacity: 0.85,
                          fill: false,
                          className: "aoi-overlay-halo",
                        }}
                        interactive={false}
                      />
                      <Rectangle bounds={bounds!} pathOptions={pathOptions} interactive={false} />
                    </Fragment>
                  )}
                />
                {smokeAoiBbox && (
                  <AoiOverlay
                    geometry={null}
                    bbox={smokeAoiBbox}
                    variant="smoke"
                    suppressInteriorFill={hasActiveRasterOverlay}
                    renderPolygon={() => null}
                    renderRectangle={({ key, bounds, pathOptions }) => (
                      <Rectangle key={key} bounds={bounds!} pathOptions={pathOptions} interactive={false} />
                    )}
                  />
                )}
              </>
            );
          })()}
        </Pane>
        {!drawActive && aoiOverlayRendered && (
          <AoiOverlay
            geometry={aoiGeometry}
            renderGeometry={renderedAoi}
            bbox={aoiOverlayBbox}
            renderPolygon={({ key, positions, pathOptions }) => (
              <Polygon
                key={`aoi-hover:${key}`}
                positions={positions!}
                pathOptions={{
                  ...pathOptions,
                  color: "#0f766e",
                  opacity: 0.01,
                  weight: 14,
                  fill: false,
                  fillOpacity: 0,
                  className: "aoi-outline-hit-layer",
                }}
                bubblingMouseEvents={false}
                eventHandlers={{
                  mouseover: handleAoiTooltipShow,
                  mousemove: handleAoiTooltipShow,
                  mouseout: handleAoiTooltipHide,
                }}
              />
            )}
            renderRectangle={({ key, bounds, pathOptions }) => (
              <Rectangle
                key={`aoi-hover:${key}`}
                bounds={bounds!}
                pathOptions={{
                  ...pathOptions,
                  color: "#0f766e",
                  opacity: 0.01,
                  weight: 14,
                  fill: false,
                  fillOpacity: 0,
                  className: "aoi-outline-hit-layer",
                }}
                bubblingMouseEvents={false}
                eventHandlers={{
                  mouseover: handleAoiTooltipShow,
                  mousemove: handleAoiTooltipShow,
                  mouseout: handleAoiTooltipHide,
                }}
              />
            )}
          />
        )}
        {orderedSceneEntries.map(({ scene, footprint, index }) => {
          const identity = sceneIdentity(scene);
          return (
            <MapSceneFootprintEntry
              key={identity}
              scene={scene}
              footprint={footprint}
              index={index}
              selected={identity === selectedSceneId}
              hovered={identity === effectiveHoveredSceneId || identity === hoveredSceneId}
              drawActive={drawActive}
              onSceneSelect={handleSceneSelect}
              onTooltipShow={handleSceneTooltipShow}
              onTooltipHide={handleSceneTooltipHide}
            />
          );
        })}
        <DrawLayer
          mode={drawMode}
          active={!drawDisabled && drawActive}
          onRect={onRectDraw}
          onPolygon={onPolygonDraw}
          onPoint={onPointDraw}
        />
        <GeeMapLeafletOverlays
          swipeLeftLayer={swipeLeftLayer}
          swipeRightLayer={swipeRightLayer}
          swipePosition={swipePosition}
        />
        <UiSmokeMapChecks aoiOverlayRendered={aoiOverlayRendered} />
        <MapStatusOverlay chrome={chrome} declutter={declutter} mapStatus={mapStatus} containerWidth={containerWidth} />
      </MapContainer>

      <GeeLayerStackHud
        swipeLeftLayer={swipeLeftLayer}
        swipeRightLayer={swipeRightLayer}
        swipePosition={swipePosition}
        onSwipePositionChange={onSwipePositionChange}
        onClearSwipe={onClearSwipe}
      />

      <BoundedSceneTooltip
        map={mapApi}
        state={disableSceneHoverCard || drawActive ? null : sceneTooltip}
        heading={orbitLabels?.heading}
        popupTitleMode={popupTitleMode}
        hideDownloadEntry={hideDownloadEntry}
        stackSize={
          sceneTooltip
            ? (stackSizeFor?.(sceneTooltip.scene.scene_id) ??
               stackScenesFor?.(sceneTooltip.scene.scene_id)?.length)
            : undefined
        }
      />
      <BoundedAoiTooltip
        map={mapApi}
        state={drawActive || sceneTooltip ? null : aoiTooltip}
        name={aoiName}
        kind={aoiKind}
        areaKm2={aoiAreaKm2}
      />

      <MapProviderStatusOverlay status={mapStatus} chrome={chrome} declutter={declutter} />

      {(chrome === "panel" || Boolean(activeStackScenes)) && (
        <div className={cn("pointer-events-none absolute z-[550] flex flex-col items-start gap-2 max-w-[min(32rem,calc(100%-2rem))]", topOverlayOffsetClass, declutter ? "left-12" : "left-3")}>
          {chrome === "panel" && footprintCount > 0 && !declutter && (
            <div className="pointer-events-auto rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-1 text-[12px] text-sp-text-muted shadow-overlay select-none">
              {t("panels.map.footprintHoverHint")}
            </div>
          )}
          {activeStackScenes && (
            <div
              ref={stackPopoverRef}
              role="dialog"
              aria-label={t("panels.map.sameTrackList")}
              className="pointer-events-auto rounded-sp-md border border-sp-line bg-sp-surface shadow-overlay overflow-hidden min-w-[290px] max-w-[380px]"
            >
              <div className="flex items-center justify-between px-3 py-2 border-b border-sp-line bg-sp-surface-muted select-none">
                <span className="text-[13px] font-semibold text-sp-text">
                  {t("panels.map.sameTrackStack", { scenes: activeStackScenes.length })}
                </span>
                <button
                  type="button"
                  onClick={() => {
                    setMapHoveredSceneId(null);
                    setActiveStackScenes(null);
                  }}
                  aria-label={t("panels.map.close")}
                  title={t("panels.map.close")}
                  className="w-5 h-5 flex items-center justify-center rounded-sp-sm text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer"
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </div>
              <div
                className="max-h-[220px] overflow-y-auto"
                onMouseLeave={() => setMapHoveredSceneId(null)}
              >
                {activeStackScenes.map((item) => {
                  const itemKey = sceneIdentity(item);
                  const isItemSelected = itemKey === selectedSceneId;
                  const isItemHovered = itemKey === effectiveHoveredSceneId;
                  const dateStr = item.acquisition_datetime?.slice(0, 10) || "-";
                  const hasOrbitDir = Boolean(item.orbit_direction?.trim());
                  const orbitDir = hasOrbitDir ? formatOrbitDirection(item.orbit_direction) : "";
                  const stackShortTag = describeSceneStack(item).shortTag;
                  const rawCloud = extractCloudCover(item);
                  return (
                    <button
                      key={item.scene_id}
                      type="button"
                      onMouseEnter={() => setMapHoveredSceneId(itemKey)}
                      onFocus={() => setMapHoveredSceneId(itemKey)}
                      onClick={() => {
                        setMapHoveredSceneId(null);
                        onSceneSelectRef.current?.(itemKey);
                      }}
                      title={sceneListLabel(item)}
                      className={cn(
                        "w-full text-left px-3 py-1.5 flex items-center justify-between gap-2 border-b last:border-b-0 border-sp-line text-[13px] hover:bg-sp-hover cursor-pointer transition-colors",
                        isItemSelected && "bg-sp-selected font-medium",
                        isItemHovered && !isItemSelected && "bg-amber-500/10",
                      )}
                    >
                      <div className="flex items-center gap-2 min-w-0">
                        <span
                          className={cn(
                            "w-2 h-2 rounded-full shrink-0 transition-transform",
                            isItemSelected
                              ? "bg-sp-accent scale-125"
                              : isItemHovered
                                ? "bg-amber-400 scale-125"
                                : "bg-sp-line",
                          )}
                        />
                        <span className="tabular-nums text-sp-text font-medium shrink-0">{dateStr}</span>
                        {hasOrbitDir && orbitDir && (
                          <span className="text-sp-text-secondary shrink-0">{orbitDir}</span>
                        )}
                        {stackShortTag && (
                          <span className="text-[11px] font-mono text-sp-text-secondary shrink-0">
                            {stackShortTag}
                          </span>
                        )}
                        {rawCloud !== null && (
                          <span className="text-[11px] text-sp-text-secondary shrink-0 tabular-nums">
                            {t("panels.map.cloudPctShort", { cloud: rawCloud.toFixed(0) })}
                          </span>
                        )}
                        <span className="font-mono text-sp-text-muted truncate">
                          {truncateMiddle(sceneListLabel(item), 24)}
                        </span>
                      </div>
                      {isItemSelected && (
                        <span className="text-[11px] font-semibold text-sp-accent shrink-0">{t("panels.map.selectedBadge")}</span>
                      )}
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}

      {!declutter && showTileStatusOverlay && previewTileStatus && (
        <div
          data-catalog-tile-preview-status={previewTileStatus}
          role={previewTileStatus === "failed" ? "alert" : "status"}
          className={cn(
            "pointer-events-none absolute bottom-[6.75rem] left-3 z-[530] max-w-[min(28rem,calc(100%-1.5rem))] rounded-lg border px-3 py-2 text-xs shadow-lg backdrop-blur-xl",
            previewTileStatus === "failed"
              ? "border-destructive/40 bg-background/92 text-destructive"
              : "border-border/70 bg-background/86 text-foreground",
          )}
        >
          {previewTileStatus === "ready"
            ? t("panels.map.previewTilesReady", { layer: previewTileLayer?.statusLabel ?? t("panels.map.trueColorTiles") })
            : previewTileStatus === "failed"
              ? t("panels.map.previewTilesFailed", { layer: previewTileLayer?.statusLabel ?? t("panels.map.trueColorTiles") })
              : t("panels.map.previewTilesLoading", { layer: previewTileLayer?.statusLabel ?? t("panels.map.trueColorTiles") })}
        </div>
      )}

      {(chrome === "default" || declutter) && (
        <MapToolbar
          map={mapApi}
          drawMode={drawMode}
          drawActive={!drawDisabled && drawActive}
          drawDisabled={drawDisabled}
          layersOpen={layersOpen}
          onDrawModeChange={onDrawModeChange}
          onDrawActiveChange={onDrawActiveChange}
          onClearLayers={onClearLayers}
          onToggleLayers={() => setLayersOpen((value) => !value)}
          declutter={declutter}
        />
      )}

      {chrome === "panel" && !declutter && geodataOverlayToggleChip && (
        <div className="pointer-events-auto absolute right-14 top-3 z-[500] flex items-center gap-1.5">
          {geodataOverlayToggleChip}
        </div>
      )}

      {chrome === "panel" && !declutter && (
        <div
          className="pointer-events-auto absolute right-3 top-3 z-[500] flex flex-col gap-1 bg-sp-surface rounded-sp-md shadow-overlay border border-sp-line p-1"
          onPointerDown={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
        >
          <button
            type="button"
            title={t("panels.map.zoomIn")}
            aria-label={t("panels.map.zoomIn")}
            onClick={() => mapApi?.zoomIn()}
            className="w-8 h-8 rounded-sp-sm flex items-center justify-center text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer transition-colors"
          >
            <Plus className="w-4 h-4" />
          </button>
          <button
            type="button"
            title={t("panels.map.zoomOut")}
            aria-label={t("panels.map.zoomOut")}
            onClick={() => mapApi?.zoomOut()}
            className="w-8 h-8 rounded-sp-sm flex items-center justify-center text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer transition-colors"
          >
            <Minus className="w-4 h-4" />
          </button>
          <button
            type="button"
            title={t("panels.map.switchBasemap")}
            aria-label={t("panels.map.switchBasemap")}
            aria-pressed={layersOpen}
            onClick={() => setLayersOpen((value) => !value)}
            className={cn(
              "w-8 h-8 rounded-sp-sm flex items-center justify-center text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer transition-colors",
              layersOpen && "bg-sp-hover text-sp-text"
            )}
          >
            <Layers className="w-4 h-4" />
          </button>
        </div>
      )}

      {chrome === "panel" && !declutter && (
        <div
          className="pointer-events-none absolute left-3 bottom-3 z-[500] flex items-center gap-3 bg-sp-surface rounded-sp-md shadow-overlay border border-sp-line px-3 py-1 text-xs text-sp-text-secondary select-none flex-wrap max-w-[calc(100%-1.5rem)]"
        >
          <span className="inline-flex items-center gap-1.5 shrink-0">
            <span className="w-2 h-2 rounded-full" style={{ background: MAP_ASC_COLOR }} />
            <span>Ascending</span>
          </span>
          <span className="inline-flex items-center gap-1.5 shrink-0">
            <span className="w-2 h-2 rounded-full" style={{ background: MAP_DESC_COLOR }} />
            <span>Descending</span>
          </span>
          {Boolean((aoiBbox || aoiGeometry) && aoiName) && (
            <span className="inline-flex items-center gap-1 shrink-0 truncate max-w-[12rem]" title={aoiName}>
              <SquareDashed className="w-3.5 h-3.5 text-sp-text shrink-0" />
              <span className="truncate">{aoiName}</span>
            </span>
          )}
        </div>
      )}

      {declutter ? (
        (!hideDefaultLegend && declutterLegendItems.length > 0) || topControls || geodataOverlayToggleChip ? (
          <div
            data-testid={verticalTopRightControls ? "agent-map-vertical-toolbar" : "map-declutter-top-right-dock"}
            className={cn(
              "pointer-events-auto absolute right-3 z-[500] flex",
              verticalTopRightControls ? "flex-col items-end gap-1.5" : "items-start gap-1.5",
              topOverlayOffsetClass,
            )}
          >
            {!hideDefaultLegend && declutterLegendItems.length > 0 && (
              <div
                ref={legendPopoverRef}
                data-testid="map-dock-legend"
                data-legend-state={legendOpen ? "open" : "closed"}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => e.stopPropagation()}
                style={
                  {
                    "--sp-dock-closed-w": `${dockMetrics.closedW}px`,
                    "--sp-dock-open-w": `${dockMetrics.openW}px`,
                    "--sp-dock-open-h": `${dockMetrics.openH}px`,
                  } as React.CSSProperties
                }
                className={cn(
                  "sp-dock-legend select-none pointer-events-auto",
                  legendOpen && "is-open",
                )}
              >
                <button
                  ref={legendButtonRef}
                  type="button"
                  data-testid="map-dock-legend-toggle"
                  title={t("panels.map.legend")}
                  aria-label={t("panels.map.legend")}
                  aria-expanded={legendOpen}
                  onClick={(e) => {
                    e.stopPropagation();
                    setLegendOpen((prev) => !prev);
                  }}
                  className={cn(
                    "sp-dock-trigger flex h-7 shrink-0 w-full items-center justify-between gap-2 px-2.5 text-xs text-sp-text-secondary hover:text-sp-text cursor-pointer select-none outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
                    legendOpen && "text-sp-text font-medium",
                  )}
                >
                  <span ref={legendTriggerLeftRef} className="flex items-center gap-1.5 min-w-0">
                    <ListTree className="w-3.5 h-3.5 shrink-0" />
                    <span className="whitespace-nowrap">{t("panels.map.legend")}</span>
                    <span className="sp-dock-dots" aria-hidden="true">
                      {declutterLegendItems.map((item) => (
                        <Fragment key={`dot-${item.key}`}>{item.dot}</Fragment>
                      ))}
                    </span>
                  </span>
                  <span className="sp-dock-toggle-wrap shrink-0" aria-hidden="true">
                    <span className="sp-dock-toggle-bg" />
                    <ChevronDown className="sp-dock-chevron w-3.5 h-3.5" />
                  </span>
                </button>

                <div
                  className="sp-dock-grid-wrap"
                  aria-hidden={!legendOpen}
                >
                  <div className="sp-dock-grid-inner">
                    <div
                      ref={legendGridContentRef}
                      className={cn(
                        "grid gap-x-2 gap-y-0.5 px-2 pb-1.5 pt-1 text-xs w-max max-w-[20rem] border-t border-sp-divider",
                        declutterLegendItems.length === 1
                          ? "grid-cols-1"
                          : "grid-cols-[max-content_max-content]"
                      )}
                    >
                      {declutterLegendItems.map((item, idx) => (
                        <div
                          key={item.key}
                          className="sp-dock-item"
                          style={{ "--sp-dock-idx": idx } as React.CSSProperties}
                          title={item.label}
                        >
                          <span className="sp-dock-item-bg" />
                          <span className="relative z-10 flex items-center gap-2 min-w-0">
                            {item.dot}
                            <span className="truncate text-sp-text">{item.label}</span>
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              </div>
            )}
            {geodataOverlayToggleChip}
            {topControls}
          </div>
        ) : null
      ) : chrome === "default" ? (
        <div className="pointer-events-none absolute right-3 top-3 z-[500] max-w-[min(44rem,calc(100%-5rem))] overflow-hidden rounded-sp-sm border border-sp-line bg-sp-surface/95 px-2.5 py-1.5 text-[11px] text-sp-text shadow-overlay">
          <div className="flex min-w-0 flex-nowrap items-center justify-start gap-2 whitespace-nowrap">
            <span className="inline-flex shrink-0 items-center gap-1.5 font-medium">
              <MapPinned className="h-4 w-4 text-primary" />
              <span className="shrink-0">{drawActive ? t("panels.map.drawingAoi") : t("panels.map.mapWorkspace")}</span>
            </span>
            {geodataOverlayToggleChip}
            {aoiOverlayRendered && (
              <span className="hidden shrink-0 items-center gap-1 text-amber-700 md:inline-flex dark:text-amber-300">
                <span className="h-2 w-2 rounded-full bg-[#eab308]" />
                {t("panels.map.legendAoiBoundary")}
              </span>
            )}
            {renderedDemEnvelopeBbox && (
              <span className="hidden shrink-0 items-center gap-1 text-amber-700 md:inline-flex dark:text-amber-300">
                <span className="h-2.5 w-2.5 rounded-full bg-amber-500 ring-1 ring-amber-700/60" />
                {t("panels.map.legendDemExtent")}
              </span>
            )}
            <button
              type="button"
              disabled={isKmlCopyDisabled}
              className={cn(
                "pointer-events-auto min-w-0 max-w-[14rem] sm:max-w-none flex-1 truncate rounded-md px-3 py-1.5 text-left font-mono text-[12px] text-foreground tabular-nums transition-colors",
                !isKmlCopyDisabled
                  ? "hover:bg-white/70 hover:text-primary dark:hover:bg-white/10 cursor-pointer"
                  : "opacity-60 cursor-default",
              )}
              onClick={copyFitBbox}
              title={
                isKmlCopyDisabled
                  ? t("panels.map.noWorkspaceAoi")
                  : aoiKmlPolygons.length > 0
                  ? t("panels.map.copyAoiKml")
                  : t("panels.map.copyAoiBboxKml")
              }
            >
              {bboxCopied ? t("panels.map.kmlCopied") : displaySummaryLabel}
            </button>
            {footprintCount > 0 && hasOrbitDirection && (
              <>
                <span className="hidden shrink-0 items-center gap-1 md:inline-flex">
                  <span className="h-2 w-2 rounded-full" style={{ background: MAP_ASC_COLOR }} />
                  {orbitLabels?.ascending ?? t("panels.map.ascending")}
                </span>
                <span className="hidden shrink-0 items-center gap-1 md:inline-flex">
                  <span className="h-2 w-2 rounded-full" style={{ background: MAP_DESC_COLOR }} />
                  {orbitLabels?.descending ?? t("panels.map.descending")}
                </span>
                <span className="hidden shrink-0 items-center gap-1 md:inline-flex">
                  <span className="h-2 w-2 rounded-full bg-[#64748b]" />
                  {orbitLabels?.unknown ?? t("panels.map.unknown")}
                </span>
              </>
            )}
          </div>
        </div>
      ) : null}

      {declutter && (
        <div
          ref={attributionRef}
          onMouseEnter={() => setAttributionHovered(true)}
          onMouseLeave={() => setAttributionHovered(false)}
          className="pointer-events-auto absolute right-3 bottom-3 z-[520] flex max-w-[calc(100%-1.5rem)] items-center justify-end gap-2 select-none"
        >
          {attributionOpen && (
            <div
              className={cn(
                MAP_CHIP_BASE_CLASS,
                "sp-popover-blur-in origin-right flex min-w-0 max-w-[28rem] items-center gap-1 select-none",
              )}
              title={attributionCreditsText}
            >
              <a
                href="https://leafletjs.com"
                target="_blank"
                rel="noopener noreferrer"
                className="shrink-0 text-sp-text hover:underline cursor-pointer"
                onClick={(e) => e.stopPropagation()}
              >
                Leaflet
              </a>
              {attributionLayerCredits.length > 0 && (
                <span className="min-w-0 truncate"> · {attributionLayerCredits.join(" · ")}</span>
              )}
            </div>
          )}
          <button
            ref={attributionButtonRef}
            type="button"
            title={t("panels.map.mapAttribution")}
            aria-label={t("panels.map.mapAttribution")}
            aria-expanded={attributionOpen}
            onClick={(e) => {
              e.stopPropagation();
              setAttributionOpen((prev) => !prev);
            }}
            className={cn(
              MAP_CHIP_ICON_CLASS,
              "pointer-events-auto cursor-pointer transition-colors duration-fast ease-sp hover:text-sp-text hover:bg-sp-surface",
              attributionOpen && "bg-sp-surface text-sp-text",
            )}
          >
            <Info className="w-4 h-4" />
          </button>
        </div>
      )}

      {layersOpen && (
        <div
          ref={layerBackdropRef}
          className="pointer-events-auto absolute inset-0 z-[640]"
          style={{ zIndex: 640 }}
          onPointerDown={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setLayersOpen(false);
          }}
          onPointerUp={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
          onMouseUp={(event) => event.stopPropagation()}
          onMouseMove={(event) => event.stopPropagation()}
          onWheel={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onDoubleClick={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.stopPropagation()}
        />
      )}

      {layersOpen && (
        <div
          ref={layerPanelRef}
          className={cn(
            "sp-popover-blur-in pointer-events-auto absolute z-[790] flex max-h-[calc(100%-2rem)] w-[290px] flex-col overflow-hidden rounded-sp-md border border-sp-line bg-sp-surface p-2.5 text-sp-text shadow-modal",
            chrome === "panel" && !declutter ? "right-3 top-14 origin-top-right" : "left-16 top-4 origin-top-left"
          )}
          style={{ zIndex: 790 }}
          onPointerDown={(event) => event.stopPropagation()}
          onPointerUp={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
          onMouseUp={(event) => event.stopPropagation()}
          onMouseMove={(event) => event.stopPropagation()}
          onWheel={(event) => event.stopPropagation()}
          onClick={(event) => event.stopPropagation()}
          onDoubleClick={(event) => event.stopPropagation()}
          onContextMenu={(event) => event.stopPropagation()}
        >
          <div className="flex shrink-0 items-center gap-2 px-1.5 pb-2 text-xs font-semibold text-sp-text">
            <Layers className="h-3.5 w-3.5 text-sp-accent" />
            {t("panels.map.mapLayers")}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto pr-0.5">
          <div className="px-1.5 pb-1 text-xs font-semibold text-sp-text-muted"><span>{t("panels.map.basemap")}</span></div>
          <div className="space-y-1">
            {MAP_PROVIDERS.map((provider) => {
              const item = MAP_LAYERS[provider.key];
              const active = provider.key === effectiveLayerKey;
              const locked =
                provider.production_available !== true
                || !provider.enabled_in_preview
                || (provider.requires_token && !hasTiandituToken);
              const description = provider.requires_token && !hasTiandituToken
                ? t("panels.map.tokenRequired")
                : !provider.enabled_in_preview
                  ? t("panels.map.previewDisabled")
                  : item.desc;
              const tooltipText = item.category
                ? `${item.label} · ${description}（${item.category}）`
                : `${item.label} · ${description}`;
              return (
                <button
                  key={provider.key}
                  type="button"
                  title={tooltipText}
                  disabled={locked}
                  onPointerDown={(event) => event.stopPropagation()}
                  onPointerUp={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    if (locked) return;
                    if (active) {
                      setLayersOpen(false);
                      return;
                    }
                    window.requestAnimationFrame(() => onLayerChange(provider.key));
                  }}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-sp-sm border px-2 py-1.5 text-left text-xs transition-colors disabled:cursor-default",
                    active
                      ? "border-sp-accent/30 bg-sp-selected text-sp-text"
                      : "border-transparent hover:bg-sp-hover",
                    locked && "opacity-50",
                  )}
                >
                  <span
                    className={cn(
                      "flex h-4 w-4 shrink-0 items-center justify-center rounded-full border",
                      active
                        ? "border-sp-accent bg-sp-accent-fill text-white"
                        : "border-sp-line",
                    )}
                  >
                    {active && <Check className="h-3 w-3" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{item.label}</span>
                    <span
                      className={cn(
                        "block truncate text-[11px]",
                        active ? "text-sp-text-secondary" : "text-sp-text-muted",
                      )}
                    >
                      {description}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
          <div className="mt-3 border-t border-sp-line px-1.5 pb-1 pt-2 text-xs font-semibold text-sp-text-muted"><span>{t("panels.map.annotation")}</span></div>
          <div className="space-y-1">
            {MAP_ANNOTATION_PROVIDERS.map((annotation) => {
              const preferenceKey = annotation.z_index_role === "imageAnnotation" ? "image" : "vector";
              const preferred = annotationPreference[preferenceKey];
              const effective = effectiveAnnotations[preferenceKey];
              const lockedByLabels = layer.containsLabels;
              const locked = !hasTiandituToken || lockedByLabels;
              const description = !hasTiandituToken
                ? t("panels.map.tokenRequired")
                : lockedByLabels
                  ? t("panels.map.annotationSuppressed")
                  : effective ? t("panels.map.annotationShown") : t("panels.map.annotationHidden");
              const annotationLabel = ANNOTATION_LABEL_KEYS[annotation.key]
                ? t(ANNOTATION_LABEL_KEYS[annotation.key])
                : annotation.display_name;
              return (
                <button
                  key={annotation.key}
                  type="button"
                  role="checkbox"
                  aria-checked={preferred}
                  title={`${annotationLabel} · ${description}`}
                  disabled={locked}
                  onPointerDown={(event) => event.stopPropagation()}
                  onPointerUp={(event) => event.stopPropagation()}
                  onClick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    if (locked) return;
                    setAnnotationPreference((value) => ({
                      ...value,
                      [preferenceKey]: !value[preferenceKey],
                    }));
                  }}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-sp-sm border px-2 py-1.5 text-left text-xs transition-colors disabled:cursor-default",
                    effective
                      ? "border-sp-accent/30 bg-sp-selected text-sp-text"
                      : "border-transparent hover:bg-sp-hover",
                    locked && "opacity-50",
                  )}
                >
                  <span className={cn(
                    "flex h-4 w-4 shrink-0 items-center justify-center rounded border",
                    preferred ? "border-sp-accent bg-sp-accent-fill text-white" : "border-sp-line",
                  )}>
                    {preferred && <Check className="h-3 w-3" />}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{annotationLabel}</span>
                    <span className="block truncate text-[11px] text-sp-text-muted">{description}</span>
                  </span>
                </button>
              );
            })}
          </div>
          <div className="mt-3 border-t border-sp-line px-1.5 pt-2 text-[11px] leading-5 text-sp-text-muted">
            {t("panels.map.tiandituNote")}
          </div>
          </div>
          <div className="mt-2 shrink-0 border-t border-sp-line px-1 pt-2">
            <button
              type="button"
              data-testid="map-open-tile-download-workbench-btn"
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                const resolvedCurrentKey =
                  effectiveLayerKey === "cartoDark"
                    ? "carto_dark"
                    : effectiveLayerKey === "cartoLight"
                      ? "carto_light"
                      : effectiveLayerKey === "osm"
                        ? "osm"
                        : effectiveLayerKey === "openTopoMap"
                          ? "opentopomap"
                          : effectiveLayerKey === "arcgisTopo"
                            ? "arcgis_topo"
                            : effectiveLayerKey === "arcgisStreet"
                              ? "arcgis_street"
                              : effectiveLayerKey === "googleHybrid"
                                ? "google_hybrid"
                                : effectiveLayerKey === "googleMap"
                                  ? "google_map"
                                  : effectiveLayerKey === "googleSatellite"
                                    ? "google_satellite"
                                    : effectiveLayerKey === "gaodeMap"
                                      ? "gaode_map"
                                      : effectiveLayerKey === "gaodeSatellite"
                                        ? "gaode_satellite"
                                        : effectiveLayerKey === "tiandituVector"
                                          ? "tianditu_vector"
                                          : effectiveLayerKey === "tiandituSatellite"
                                            ? "tianditu_satellite"
                                            : "arcgis_satellite";
                setLayersOpen(false);
                onOpenTileDownloadWorkbench?.(resolvedCurrentKey);
              }}
              className="flex w-full items-center justify-between gap-2 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2.5 py-2 text-left text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer transition-colors"
            >
              <span>{t("panels.map.openTileDownloadWorkbench")}</span>
              <ArrowUpRight className="h-3.5 w-3.5 shrink-0 text-sp-text-secondary" />
            </button>
          </div>
        </div>
      )}

      {(htmlVectorDrag || nativeVectorDrag || vectorImportBusy) && (
        <div
          data-testid="workbench-map-vector-drop-overlay"
          className="pointer-events-none absolute inset-0 z-[1200] flex items-center justify-center bg-sp-surface/75 backdrop-blur-xs border-2 border-dashed border-sp-accent p-6 text-center"
        >
          <div className="flex flex-col items-center gap-2 rounded-xl border border-sp-line bg-sp-surface/95 px-5 py-3.5 shadow-card text-xs font-medium text-sp-text">
            <Upload className="h-5 w-5 text-sp-accent" />
            <span>
              {vectorImportBusy
                ? t("panels.map.parsingVectorAoi")
                : t("panels.map.dropVectorAoiHint")}
            </span>
          </div>
        </div>
      )}

      {vectorDropError && !htmlVectorDrag && !nativeVectorDrag && (
        <div className="pointer-events-auto absolute top-14 left-1/2 -translate-x-1/2 z-[1200] flex items-center gap-2 rounded-lg border border-sp-danger-text/30 bg-sp-surface/95 px-3 py-1.5 text-xs text-sp-danger-text shadow-card max-w-[90%]">
          <span className="truncate">{vectorDropError}</span>
          <button
            type="button"
            onClick={() => setVectorDropError(null)}
            className="shrink-0 text-sp-text-muted hover:text-sp-text cursor-pointer"
          >
            {t("panels.map.close")}
          </button>
        </div>
      )}

    </div>
  );
}

export const WorkbenchMap = memo(WorkbenchMapComponent);
