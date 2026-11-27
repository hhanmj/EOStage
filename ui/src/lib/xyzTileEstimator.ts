import type { Bbox } from "./bridge";

export type XyzExportFormat =
  | "gtiff"
  | "cog"
  | "mbtiles"
  | "zip"
  | "png"
  | "jpg"
  | "envi"
  | "vrt";

export type XyzCompression = "NONE" | "DEFLATE" | "LZW" | "JPEG" | "WEBP";

export interface XyzZoomTileSummary {
  zoom: number;
  xMin: number;
  xMax: number;
  yMin: number;
  yMax: number;
  cols: number;
  rows: number;
  tileCount: number;
  widthPx: number;
  heightPx: number;
  networkDownloadBytes: number;
  uncompressedBytes: number;
  estimatedOutputBytes: number;
}

export interface XyzTileEstimate {
  bbox: Bbox;
  zooms: number[];
  perZoom: XyzZoomTileSummary[];
  totalTiles: number;
  maxWidthPx: number;
  maxHeightPx: number;
  maxGridWidthPx: number;
  maxGridHeightPx: number;
  totalNetworkDownloadBytes: number;
  totalUncompressedBytes: number;
  totalEstimatedOutputBytes: number;
  estimatedPeakMemoryBytes: number;
  needsCogStagingDisk: boolean;
  maxZoom: number;
  maxTilesLimit: number;
  exceedsSourceMaxZoom: boolean;
  exceedsHardLimit: boolean;
  exceedsSafeLimit: boolean;
  exceedsJpegDimensionLimit: boolean;
}

const TILE_PX = 256;
const MIN_LAT = -85.0511;
const MAX_LAT = 85.0511;
const AVG_TILE_BYTES = 28 * 1024; // ~28 KB per compressed web tile (PNG/JPEG/WebP mix)

export const XYZ_MAX_TILES_MOSAIC = 50_000;
export const XYZ_MAX_TILES_ARCHIVE = 200_000;
export const XYZ_JPEG_MAX_DIMENSION = 65_535;
export const SAFE_TILE_LIMIT = XYZ_MAX_TILES_MOSAIC;

export const XYZ_SOURCE_MAX_ZOOM: Record<string, number> = {
  carto_light: 16,
  cartolight: 16,
  carto_dark: 16,
  cartodark: 16,
  opentopomap: 17,
  opentopo: 17,
  tianditu_satellite: 18,
  tianditusatellite: 18,
  tianditu_img: 18,
  tianditu_vector: 18,
  tiandituvector: 18,
  tianditu_vec: 18,
  gaode_satellite: 18,
  gaodesatellite: 18,
  gaode_sat: 18,
  gaode: 18,
  autonavi_satellite: 18,
  amap_satellite: 18,
  amap: 18,
  gaode_map: 18,
  gaodemap: 18,
  gaode_vector: 18,
  gaode_vec: 18,
  autonavi_vector: 18,
  amap_vector: 18,
  osm: 19,
  openstreetmap: 19,
  arcgis_topo: 19,
  arcgistopo: 19,
  esri_topo: 19,
  arcgis_street: 19,
  arcgisstreet: 19,
  esri_street: 19,
  arcgis_satellite: 20,
  arcgissatellite: 20,
  esri_satellite: 20,
  esri: 20,
  esri_wayback: 20,
  google_satellite: 21,
  googlesatellite: 21,
  google_sat: 21,
  google: 21,
  google_hybrid: 21,
  googlehybrid: 21,
  google_map: 21,
  googlemap: 21,
  google_roadmap: 21,
  mt1: 21,
  custom: 22,
  custom_xyz: 22,
  custom_tms: 22,
};

export function getXyzSourceMaxZoom(source?: string | null): number {
  const key = (source || "").trim().toLowerCase();
  if (/^wayback_\d{1,16}$/.test(key)) {
    return 20;
  }
  return XYZ_SOURCE_MAX_ZOOM[key] ?? 22;
}

export function getXyzMaxTilesLimit(exportFormat?: string | null, stitchMosaic = true): number {
  const fmt = (exportFormat || "gtiff").trim().toLowerCase();
  const requiresRasterStitchOrVrt = (stitchMosaic && fmt !== "mbtiles") || fmt === "vrt";
  return requiresRasterStitchOrVrt ? XYZ_MAX_TILES_MOSAIC : XYZ_MAX_TILES_ARCHIVE;
}

export function normalizeXyzZooms(
  zooms: readonly number[] | null | undefined,
  fallbackZoom: number | null = 15,
  maxZoomOrSource?: number | string | null,
): number[] {
  const ceiling =
    typeof maxZoomOrSource === "number"
      ? Math.min(22, Math.max(1, Math.round(maxZoomOrSource)))
      : typeof maxZoomOrSource === "string"
        ? getXyzSourceMaxZoom(maxZoomOrSource)
        : 22;
  const clean = new Set<number>();
  if (Array.isArray(zooms)) {
    for (const raw of zooms) {
      const z = Math.round(Number(raw));
      if (Number.isFinite(z) && z >= 1 && z <= ceiling) {
        clean.add(z);
      }
    }
  }
  if (clean.size === 0 && fallbackZoom !== null && Number.isFinite(fallbackZoom)) {
    const z = Math.min(ceiling, Math.max(1, Math.round(fallbackZoom)));
    clean.add(z);
  }
  return Array.from(clean).sort((a, b) => a - b);
}

export function lon2tile(lon: number, zoom: number): number {
  const n = Math.pow(2, zoom);
  const clampedLon = Math.min(179.9999, Math.max(-179.9999, lon));
  return Math.min(n - 1, Math.max(0, Math.floor(((clampedLon + 180) / 360) * n)));
}

export function lat2tile(lat: number, zoom: number): number {
  const n = Math.pow(2, zoom);
  const clampedLat = Math.min(MAX_LAT, Math.max(MIN_LAT, lat));
  const latRad = (clampedLat * Math.PI) / 180;
  const y = Math.floor(
    ((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n,
  );
  return Math.min(n - 1, Math.max(0, y));
}

function outputCompressionRatio(format: string, compression: string): number {
  const fmt = format.toLowerCase();
  const comp = compression.toUpperCase();
  if (fmt === "vrt") return 0.0005;
  if (fmt === "mbtiles" || fmt === "zip") {
    // Tile-archive formats store original compressed web tiles directly (+ small SQLite/ZIP index overhead)
    return 0;
  }
  if (fmt === "jpg" || fmt === "jpeg" || comp === "JPEG" || comp === "WEBP") return 0.14;
  if (fmt === "png") return 0.36;
  if (fmt === "envi") return 1.0;
  // GeoTIFF / COG
  if (comp === "NONE") return 1.0;
  if (comp === "DEFLATE") return 0.48;
  if (comp === "LZW") return 0.58;
  return 0.55;
}

export type EstimatorBboxInput =
  | Bbox
  | readonly [number, number, number, number];

function normalizeEstimatorBbox(
  bbox: EstimatorBboxInput | null | undefined,
): Bbox | null {
  if (!bbox) return null;
  let w: number;
  let s: number;
  let e: number;
  let n: number;
  if (Array.isArray(bbox)) {
    if (bbox.length !== 4) return null;
    [w, s, e, n] = bbox;
  } else {
    const obj = bbox as Bbox;
    w = obj.west;
    s = obj.south;
    e = obj.east;
    n = obj.north;
  }
  if (![w, s, e, n].every((v) => typeof v === "number" && Number.isFinite(v))) {
    return null;
  }
  return {
    west: Math.min(w, e),
    south: Math.min(s, n),
    east: Math.max(w, e),
    north: Math.max(s, n),
  };
}

export function estimateXyzTilesForBbox(
  bbox: EstimatorBboxInput | null | undefined,
  zoomsInput: readonly number[],
  options: {
    format?: string;
    exportFormat?: string;
    compression?: string;
    includeOverviews?: boolean;
    source?: string;
    stitchMosaic?: boolean;
    maxZoomOverride?: number | null;
  } = {},
): XyzTileEstimate | null {
  const normalizedBbox = normalizeEstimatorBbox(bbox);
  if (!normalizedBbox) {
    return null;
  }
  const { west, south, east, north } = normalizedBbox;
  const sourceMaxZoom = getXyzSourceMaxZoom(options.source);
  const maxZoom =
    typeof options.maxZoomOverride === "number" && Number.isFinite(options.maxZoomOverride)
      ? Math.min(sourceMaxZoom, Math.max(1, Math.round(options.maxZoomOverride)))
      : sourceMaxZoom;
  const rawValidZooms = Array.isArray(zoomsInput)
    ? zoomsInput.map((z) => Math.round(Number(z))).filter((z) => Number.isFinite(z))
    : [];
  const exceedsSourceMaxZoom = rawValidZooms.some((z) => z < 1 || z > maxZoom);
  const zooms = normalizeXyzZooms(zoomsInput, 15);
  const format = options.exportFormat ?? options.format ?? "gtiff";
  const fmt = format.toLowerCase();
  const stitchMosaic = options.stitchMosaic ?? true;
  const maxTilesLimit = getXyzMaxTilesLimit(format, stitchMosaic);
  const compression = options.compression ?? "DEFLATE";
  const overviewMultiplier =
    options.includeOverviews && ["gtiff", "cog", "tif", "tiff"].includes(fmt)
      ? 1.33
      : 1.0;
  const ratio = outputCompressionRatio(format, compression);

  const perZoom: XyzZoomTileSummary[] = zooms.map((zoom) => {
    const x0 = lon2tile(west, zoom);
    const x1 = lon2tile(east, zoom);
    const y0 = lat2tile(north, zoom);
    const y1 = lat2tile(south, zoom);
    const xMin = Math.min(x0, x1);
    const xMax = Math.max(x0, x1);
    const yMin = Math.min(y0, y1);
    const yMax = Math.max(y0, y1);
    const cols = xMax - xMin + 1;
    const rows = yMax - yMin + 1;
    const tileCount = cols * rows;
    const widthPx = cols * TILE_PX;
    const heightPx = rows * TILE_PX;
    const networkDownloadBytes = tileCount * AVG_TILE_BYTES;
    // RGB 3 bytes/pixel uncompressed raster size
    const uncompressedBytes = widthPx * heightPx * 3;
    const estimatedOutputBytes =
      ratio === 0
        ? Math.round(networkDownloadBytes * 1.05)
        : Math.max(4096, Math.round(uncompressedBytes * ratio * overviewMultiplier));

    return {
      zoom,
      xMin,
      xMax,
      yMin,
      yMax,
      cols,
      rows,
      tileCount,
      widthPx,
      heightPx,
      networkDownloadBytes,
      uncompressedBytes,
      estimatedOutputBytes,
    };
  });

  let totalTiles = 0;
  let maxWidthPx = 0;
  let maxHeightPx = 0;
  let totalNetworkDownloadBytes = 0;
  let totalUncompressedBytes = 0;
  let totalEstimatedOutputBytes = 0;

  for (const item of perZoom) {
    totalTiles += item.tileCount;
    if (item.widthPx > maxWidthPx) maxWidthPx = item.widthPx;
    if (item.heightPx > maxHeightPx) maxHeightPx = item.heightPx;
    totalNetworkDownloadBytes += item.networkDownloadBytes;
    totalUncompressedBytes += item.uncompressedBytes;
    totalEstimatedOutputBytes += item.estimatedOutputBytes;
  }

  const needsCogStagingDisk = fmt === "cog";
  const estimatedPeakMemoryBytes =
    fmt === "vrt"
      ? 16 * 1024 * 1024
      : fmt === "mbtiles" || fmt === "zip"
        ? 24 * 1024 * 1024
        : Math.min(128 * 1024 * 1024, Math.max(32 * 1024 * 1024, maxWidthPx * 256 * 3 * 2));

  const requiresRasterStitch = stitchMosaic && fmt !== "mbtiles" && fmt !== "vrt";
  const exceedsHardLimit = totalTiles > maxTilesLimit;
  const exceedsJpegDimensionLimit =
    requiresRasterStitch &&
    (fmt === "jpg" || fmt === "jpeg") &&
    (maxWidthPx > XYZ_JPEG_MAX_DIMENSION || maxHeightPx > XYZ_JPEG_MAX_DIMENSION);

  return {
    bbox: normalizedBbox,
    zooms,
    perZoom,
    totalTiles,
    maxWidthPx,
    maxHeightPx,
    maxGridWidthPx: maxWidthPx,
    maxGridHeightPx: maxHeightPx,
    totalNetworkDownloadBytes,
    totalUncompressedBytes,
    totalEstimatedOutputBytes,
    estimatedPeakMemoryBytes,
    needsCogStagingDisk,
    maxZoom,
    maxTilesLimit,
    exceedsSourceMaxZoom,
    exceedsHardLimit,
    exceedsSafeLimit: exceedsHardLimit,
    exceedsJpegDimensionLimit,
  };
}

export function formatTileCount(count: number): string {
  return count.toLocaleString("en-US");
}

export function formatXyzByteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 KB";
  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.max(1, Math.round(kb))} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb >= 100 ? Math.round(mb) : mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb >= 10 ? gb.toFixed(1) : gb.toFixed(2)} GB`;
}
