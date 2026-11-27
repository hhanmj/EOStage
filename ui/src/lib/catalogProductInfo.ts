import { formatByteSize } from "./formatByteSize.ts";
import { formatSignedLat, formatSignedLon } from "./geoCoords.ts";
import { getCurrentLocale, t } from "../i18n/index.ts";
import type { Locale } from "../i18n/types.ts";
import { canonicalSourceDisplayName, localizedSourceDisplayName } from "./dataSourceCatalogLogic.ts";
import type { CatalogResultRow } from "./catalogPreview.ts";
import type { SceneRow } from "./bridge.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";
import { extractCloudCover } from "../shell/api/rowProjection.ts";
import { sceneFootprintRenderModel } from "./sceneFootprint.ts";
import {
  ALOS2_DATASET_LABEL,
  alos2Properties,
  isAlos2CatalogProduct,
} from "./asfAlos2Catalog.ts";
import {
  NISAR_DATASET_LABEL,
  isNisarCatalogProduct,
  nisarProperties,
} from "./asfNisarCatalog.ts";
import {
  BHOONIDHI_SSAR_DATASET_LABEL,
  bhoonidhiPropertyText,
  bhoonidhiSsarProperties,
  bhoonidhiThumbnailAsset,
  isBhoonidhiSsarCatalogProduct,
} from "./bhoonidhiSsarCatalog.ts";
import {
  isSentinel2L1cCatalogProduct,
  isSentinel2L2aCatalogProduct,
  sentinel2CloudCover,
  sentinel2Tile,
} from "./sentinel2Catalog.ts";
import {
  isLandsatC2L2CatalogProduct,
  landsatCloudCover,
  landsatProperties,
} from "./landsatCatalog.ts";

function display(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  return String(value);
}

function fieldValue(row: CatalogResultRow, label: string): string {
  return row.domain_field_cells.find((cell) => cell.label === label)?.value || "—";
}

function hasValue(value: unknown): boolean {
  return value !== null && value !== undefined && value !== "" && value !== "—";
}

function optionalFiniteNumber(value: unknown): number {
  if (typeof value === "number") return Number.isFinite(value) ? value : Number.NaN;
  if (typeof value !== "string" || !value.trim()) return Number.NaN;
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : Number.NaN;
}

function humanBytes(value: number | null | undefined): string | null {
  return value != null && value > 0 ? formatByteSize(value, "") || null : null;
}

function properties(product: SarProduct): Record<string, unknown> {
  const native = product.native_metadata;
  const nested = native?.properties;
  return nested && typeof nested === "object" && !Array.isArray(nested)
    ? nested as Record<string, unknown>
    : {};
}

function firstProperty(product: SarProduct, keys: readonly string[]): unknown {
  const source = properties(product);
  for (const key of keys) {
    if (hasValue(source[key])) return source[key];
  }
  return null;
}

function append(lines: string[], key: string, value: unknown) {
  if (hasValue(value)) lines.push(`${key}: ${display(value)}`);
}

function coordinate(value: number, positive: string, negative: string): string {
  return `${Math.abs(value).toFixed(6)}°${value < 0 ? negative : positive}`;
}

function typedChecksum(value: string | null | undefined): [string, string] | null {
  const checksum = String(value || "").trim();
  const separator = checksum.indexOf(":");
  if (separator <= 0 || separator === checksum.length - 1) return null;
  const algorithm = checksum.slice(0, separator).trim().toLowerCase();
  const digest = checksum.slice(separator + 1).trim();
  if (!algorithm || !digest) return null;
  return [algorithm, digest];
}

export function catalogProductInfoText(
  product: SarProduct,
  row: CatalogResultRow,
): string {
  const lines: string[] = [];
  if (isLandsatC2L2CatalogProduct(product)) {
    const native = landsatProperties(product);
    append(lines, "entity_id", native.entity_id ?? product.product_id);
    append(lines, "landsat_product_id", native.landsat_product_id ?? product.scene_id);
    append(lines, "landsat_scene_id", native.landsat_scene_id);
    append(lines, "spacecraft", row.platform || product.platform);
    append(lines, "processing_level", row.processing_level || product.processing_level);
    append(lines, "data_type_l2", native.data_type_l2 ?? product.product_family);
    append(lines, "cloud_cover", fieldValue(row, "Cloud"));
    append(lines, "wrs_path_row", fieldValue(row, "WRS"));
    append(lines, "collection_category", fieldValue(row, "Category"));
    append(lines, "browse", native.browse_available === true ? "可用" : "不可用");
    append(lines, "full_bundle", fieldValue(row, "完整包"));
    append(lines, "acquisition_utc", product.acquisition_start || row.acquisition_time);
    return lines.join("\n");
  }
  if (isBhoonidhiSsarCatalogProduct(product)) {
    const native = bhoonidhiSsarProperties(product) ?? {};
    const textProperty = (...keys: string[]) => bhoonidhiPropertyText(product, ...keys);
    append(lines, "product_id", product.product_id);
    append(lines, "provider_product_key", product.provider_product_key);
    append(lines, "dataset", BHOONIDHI_SSAR_DATASET_LABEL);
    append(lines, "platform", product.platform || textProperty("Platform", "platform"));
    append(lines, "instrument", product.instrument || textProperty("Instrument", "instrument"));
    append(lines, "processing_level", product.processing_level);
    append(lines, "provider_polarization", textProperty("Polarization", "polarization"));
    append(lines, "provider_mode", textProperty("Mode", "mode"));
    append(lines, "coverage", textProperty("Coverage", "coverage"));
    append(lines, "acquisition_mode", textProperty("AcquistionMode", "AcquisitionMode", "acquisition_mode"));
    append(lines, "online", textProperty("Online", "online"));
    append(lines, "node", textProperty("Node", "node"));
    append(lines, "track", native.Track ?? product.path ?? product.relative_orbit);
    append(lines, "frame", native.Frame ?? product.frame);
    append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
    append(lines, "acquisition_end_utc", product.acquisition_stop);
    append(lines, "primary_hdf5_file", product.file_name);
    append(lines, "primary_hdf5_size", humanBytes(product.file_size_remote));
    append(lines, "primary_hdf5_size_bytes", product.file_size_remote);
    const checksum = typedChecksum(product.checksum);
    if (checksum) append(lines, checksum[0] + "_checksum", checksum[1]);
    append(
      lines,
      "preview",
      bhoonidhiThumbnailAsset(product)
        ? "thumbnail JPEG (仅显示)"
        : "unavailable (no authoritative thumbnail)",
    );
    const bbox = product.footprint_bbox;
    if (bbox) {
      append(
        lines,
        "footprint_bbox",
        "W" + bbox.west.toFixed(6) + " S" + bbox.south.toFixed(6)
          + " E" + bbox.east.toFixed(6) + " N" + bbox.north.toFixed(6),
      );
    }
    return lines.join("\n");
  }
  if (isNisarCatalogProduct(product)) {
    const native = nisarProperties(product) ?? {};
    append(lines, "product_id", product.product_id);
    append(lines, "producer_granule_id", native.producer_granule_id || product.scene_id);
    append(lines, "asf_product_id", native.asf_product_id);
    append(lines, "dataset", NISAR_DATASET_LABEL);
    append(lines, "platform", product.platform);
    append(lines, "instrument", product.instrument);
    append(lines, "processing_level", product.processing_level);
    append(lines, "science_level", native.science_level);
    append(lines, "quality_status", native.quality_status);
    append(lines, "production_configuration", native.production_configuration);
    append(lines, "band", native.band);
    append(lines, "frame_coverage", native.frame_coverage);
    append(lines, "frequency_a_polarization", native.frequency_a_polarization);
    append(lines, "frequency_b_polarization_l_band", native.frequency_b_polarization);
    append(lines, "range_bandwidth", native.range_bandwidth);
    append(
      lines,
      "joint_l_s_observation",
      `${String(native.joint_observation)} (metadata only; not an ASF S-band product)`,
    );
    append(lines, "orbit_direction", product.flight_direction);
    append(lines, "look_direction", product.look_direction);
    append(lines, "track", native.track);
    append(lines, "relative_orbit", product.relative_orbit);
    append(lines, "frame", product.frame);
    append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
    append(lines, "acquisition_end_utc", product.acquisition_stop);
    append(lines, "primary_hdf5_file", product.file_name);
    append(lines, "primary_hdf5_size", humanBytes(product.file_size_remote));
    append(lines, "primary_hdf5_size_bytes", product.file_size_remote);
    const checksum = typedChecksum(product.checksum);
    if (checksum) append(lines, `${checksum[0]}_checksum`, checksum[1]);
    append(lines, "delivery", "complete HDF5");
    append(
      lines,
      "preview",
      (product.assets ?? []).some((asset) => asset.role === "browse")
        ? "main Browse"
        : "unavailable (no authoritative main Browse)",
    );
    const bbox = product.footprint_bbox;
    if (bbox) {
      append(
        lines,
        "footprint_bbox",
        `W${bbox.west.toFixed(6)} S${bbox.south.toFixed(6)} E${bbox.east.toFixed(6)} N${bbox.north.toFixed(6)}`,
      );
    }
    return lines.join("\n");
  }
  if (isAlos2CatalogProduct(product)) {
    const native = alos2Properties(product) ?? {};
    append(lines, "product_id", product.product_id);
    append(lines, "cmr_native_id", native.cmr_native_id || product.scene_id);
    append(lines, "producer_granule_id", native.producer_granule_id);
    append(lines, "dataset", ALOS2_DATASET_LABEL);
    append(lines, "platform", product.platform);
    append(lines, "instrument", product.instrument);
    append(lines, "processing_level", product.processing_level);
    append(lines, "beam_mode", product.beam_mode);
    append(lines, "polarization", fieldValue(row, "Polarization"));
    append(lines, "orbit_direction", product.flight_direction);
    append(lines, "absolute_orbit", product.absolute_orbit);
    append(lines, "path", product.path);
    append(lines, "frame", product.frame);
    append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
    append(lines, "acquisition_end_utc", product.acquisition_stop);
    append(lines, "archive_file", product.file_name);
    append(lines, "archive_size", humanBytes(product.file_size_remote));
    append(lines, "archive_size_bytes", product.file_size_remote);
    const checksum = typedChecksum(product.checksum);
    if (checksum) append(lines, `${checksum[0]}_checksum`, checksum[1]);
    append(lines, "preview", "unavailable (no authoritative Browse)");
    const bbox = product.footprint_bbox;
    if (bbox) {
      append(
        lines,
        "footprint_bbox",
        `W${bbox.west.toFixed(6)} S${bbox.south.toFixed(6)} E${bbox.east.toFixed(6)} N${bbox.north.toFixed(6)}`,
      );
    }
    return lines.join("\n");
  }
  if (product.dataset.toUpperCase() === "SEASAT") {
    append(lines, "scene_id", product.scene_id || product.product_id);
    append(lines, "platform", product.platform || row.platform);
    append(lines, "beam_mode", product.beam_mode);
    append(lines, "polarization", product.polarization || fieldValue(row, "Polarization"));
    append(lines, "orbit_direction", product.flight_direction || fieldValue(row, "Direction"));
    append(lines, "path", product.path ?? product.relative_orbit);
    append(lines, "frame", product.frame ?? product.asf_frame);
    append(lines, "absolute_orbit", product.absolute_orbit);
    append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
    append(lines, "acquisition_end_utc", product.acquisition_stop);
    append(lines, "primary_hdf5_file", product.file_name);
    append(lines, "primary_hdf5_size", humanBytes(product.file_size_remote));
    append(lines, "primary_hdf5_size_bytes", product.file_size_remote);
    const checksum = typedChecksum(product.checksum);
    if (checksum) append(lines, `${checksum[0]}_checksum`, checksum[1]);
    else if (/^[a-f0-9]{32}$/i.test(product.checksum || "")) {
      append(lines, "md5_checksum", product.checksum);
    }
    append(
      lines,
      "preview",
      (product.assets ?? []).some((asset) => asset.role === "browse")
        ? "Browse JPEG (仅显示)"
        : "不可用",
    );
    append(
      lines,
      "derived_geotiff",
      (product.assets ?? []).some((asset) => asset.role === "derived")
        ? "可用但不自动下载"
        : "不可用",
    );
    return lines.join("\n");
  }
  if (isSentinel2L2aCatalogProduct(product)) {
    append(lines, "product_id", product.product_id);
    append(lines, "product_name", product.scene_id);
    append(lines, "platform", row.platform || product.platform);
    append(lines, "processing_level", "L2A");
    append(lines, "product_type", product.product_family);
    append(lines, "cloud_cover", fieldValue(row, "Cloud"));
    append(lines, "mgrs_tile", fieldValue(row, "Tile"));
    append(lines, "archive_availability", fieldValue(row, "Online"));
    append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
    append(lines, "acquisition_end_utc", product.acquisition_stop);
    append(lines, "archive_file", product.file_name);
    append(lines, "archive_size", humanBytes(product.file_size_remote));
    append(lines, "archive_size_bytes", product.file_size_remote);
    const checksum = typedChecksum(product.checksum);
    if (checksum) append(lines, `${checksum[0]}_checksum`, checksum[1]);
    return lines.join("\n");
  }
  append(lines, "scene_id", product.scene_id || product.product_id);
  append(lines, "platform", row.platform || product.platform);
  append(lines, "product_family", product.product_family);
  if (
    hasValue(product.processing_level)
    && String(product.processing_level).toUpperCase() !== String(product.product_family).toUpperCase()
  ) {
    append(lines, "processing_level", product.processing_level);
  }
  append(lines, "beam_mode", product.beam_mode);
  append(lines, "polarization", fieldValue(row, "Polarization"));
  append(lines, "orbit_direction", product.flight_direction || fieldValue(row, "Direction"));
  append(lines, "relative_orbit", product.relative_orbit ?? product.path);
  if (product.frame !== null && product.frame !== undefined) append(lines, "frame", product.frame);
  else append(lines, "asf_frame", product.asf_frame);
  append(lines, "absolute_orbit", product.absolute_orbit);
  append(lines, "acquisition_start_utc", product.acquisition_start || row.acquisition_time);
  append(lines, "acquisition_end_utc", product.acquisition_stop);

  append(lines, "center_incidence_angle", firstProperty(product, ["centerIncidenceAngle", "incidenceAngle"]));
  append(lines, "satellite_heading", firstProperty(product, ["satelliteHeading", "heading"]));
  append(lines, "off_nadir_angle", firstProperty(product, ["offNadirAngle"]));
  append(lines, "compressed_size", humanBytes(product.file_size_remote));
  const checksum = typedChecksum(product.checksum);
  if (checksum) append(lines, `${checksum[0]}_checksum`, checksum[1]);
  append(lines, "download_url", product.download_url);

  const bbox = product.footprint_bbox;
  if (bbox) {
    append(
      lines,
      "footprint_bbox",
      `W${bbox.west.toFixed(6)} S${bbox.south.toFixed(6)} E${bbox.east.toFixed(6)} N${bbox.north.toFixed(6)}`,
    );
  }
  const centerLatValue = firstProperty(product, ["centerLat", "centerLatitude"]);
  const centerLonValue = firstProperty(product, ["centerLon", "centerLongitude"]);
  const centerLat = optionalFiniteNumber(centerLatValue);
  const centerLon = optionalFiniteNumber(centerLonValue);
  if (
    Number.isFinite(centerLat)
    && Number.isFinite(centerLon)
    && centerLat >= -90
    && centerLat <= 90
    && centerLon >= -180
    && centerLon <= 180
  ) {
    append(
      lines,
      "footprint_centroid",
      `${coordinate(centerLon, "E", "W")}, ${coordinate(centerLat, "N", "S")}`,
    );
  }
  append(lines, "footprint_crs", bbox?.crs);
  return lines.join("\n");
}

function resolveSceneBbox(scene: SceneRow) {
  const rawBbox = scene.footprint_bbox as unknown;
  if (
    Array.isArray(rawBbox) &&
    rawBbox.length >= 4 &&
    rawBbox.slice(0, 4).every((n) => typeof n === "number" && Number.isFinite(n))
  ) {
    return {
      west: Number(rawBbox[0]),
      south: Number(rawBbox[1]),
      east: Number(rawBbox[2]),
      north: Number(rawBbox[3]),
      crs: "EPSG:4326",
    };
  }
  if (
    scene.footprint_bbox &&
    Number.isFinite(scene.footprint_bbox.west) &&
    Number.isFinite(scene.footprint_bbox.south) &&
    Number.isFinite(scene.footprint_bbox.east) &&
    Number.isFinite(scene.footprint_bbox.north)
  ) {
    return scene.footprint_bbox;
  }
  if (
    scene.sar_product?.footprint_bbox &&
    Number.isFinite(scene.sar_product.footprint_bbox.west) &&
    Number.isFinite(scene.sar_product.footprint_bbox.south) &&
    Number.isFinite(scene.sar_product.footprint_bbox.east) &&
    Number.isFinite(scene.sar_product.footprint_bbox.north)
  ) {
    return scene.sar_product.footprint_bbox;
  }
  if (scene.footprint_geojson) {
    return sceneFootprintRenderModel(scene.footprint_geojson)?.bbox ?? null;
  }
  return null;
}

function resolveSceneCloudCover(scene: SceneRow): number | null {
  const projected = extractCloudCover(scene);
  if (projected !== null) return projected;
  if (scene.sar_product) {
    const s2 = sentinel2CloudCover(scene.sar_product);
    if (s2 !== null) return s2;
    const ls = landsatCloudCover(scene.sar_product);
    if (ls !== null) return ls;
  }
  return null;
}

function resolveSceneSizeLabel(scene: SceneRow): string | null {
  const bytes =
    typeof scene.file_size_remote === "number" && scene.file_size_remote > 0
      ? scene.file_size_remote
      : scene.sar_product?.file_size_remote;
  if (typeof bytes === "number" && bytes > 0) {
    return formatByteSize(bytes, "") || null;
  }
  const mb = (scene as unknown as Record<string, unknown>).size_mb;
  if (typeof mb === "number" && mb > 0) {
    return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
  }
  return null;
}

export type SceneDisplayCategory =
  | "sar"
  | "optical"
  | "gee"
  | "orbit"
  | "event"
  | "geodesy"
  | "thematic";

export interface SceneSummaryRow {
  label: string;
  value: string;
}

export interface SceneRowMetaDisplay {
  category: SceneDisplayCategory;
  compactTime: string;
  orbitDirText: string;
  orbitDotKind: "asc" | "desc" | null;
  orbitTrackText: string | null;
  frameText: string | null;
  polarization: string | null;
  levelOrType: string | null;
  platform: string | null;
  tileOrWrs: string | null;
  cloudText: string | null;
  scaleOrSpanText: string | null;
  placeOrStation: string | null;
  sizeLabel: string | null;
  tier: "validated" | "provisional" | null;
}

const VALIDATED_NISAR_COLLECTIONS = new Set([
  "C3622236985-ASF",
  "C3622244601-ASF",
  "C3622241360-ASF",
  "C3622247503-ASF",
  "C3622250865-ASF",
  "C3622229381-ASF",
  "C3622237369-ASF",
  "C3622241997-ASF",
]);

const PROVISIONAL_NISAR_COLLECTIONS = new Set([
  "C2853145197-ASF",
  "C2854332392-ASF",
  "C2854338529-ASF",
  "C2854335566-ASF",
  "C2854341702-ASF",
  "C2853147928-ASF",
  "C2853153429-ASF",
  "C2853156054-ASF",
  "C2854344945-ASF",
]);

const READABLE_DATASET_NAMES: Readonly<Record<string, string>> = {
  "science.nisar.s.ssar.rslc.v1": "NISAR S-SAR RSLC",
  "sar.nisar.s.ssar.rslc.v1": "NISAR S-SAR RSLC",
  "science.nisar.l.rslc.provisional.v1": "NISAR L-SAR",
  "science.nisar.l.rslc.validated.v1": "NISAR L-SAR",
  "nisar_l": "NISAR L-SAR",
  "science.sentinel-1.standard.v1": "Sentinel-1",
  "sar.sentinel-1.standard": "Sentinel-1",
  "science.sentinel-1.slc-burst.v1": "Sentinel-1 SLC Burst",
  "science.aws.sentinel-1-grd.v1": "Sentinel-1 GRD",
  "aws_sentinel1_grd": "Sentinel-1 GRD",
  "science.alos-2.palsar-2.scansar-l1.1.v1": "ALOS-2 PALSAR-2 ScanSAR",
  "science.alos.palsar.v1": "ALOS PALSAR",
  "science.alos.palsar-mosaic.v1": "ALOS PALSAR 25m Mosaic",
  "alos_palsar_mosaic": "ALOS PALSAR 25m Mosaic",
  "science.seasat.sar.v1": "SEASAT SAR",
  "science.opera.disp-s1.v1": "NASA OPERA DISP-S1",
  "opera_disp_s1": "NASA OPERA DISP-S1",
  "science.aria.s1-gunw.v1": "NASA ARIA S1-GUNW",
  "aria_s1_gunw": "NASA ARIA S1-GUNW",
  "science.comet.licsar.v1": "COMET-LiCSAR",
  "comet_licsar": "COMET-LiCSAR",
  "science.sentinel-2.l2a.v1": "Sentinel-2 L2A",
  "optical.sentinel-2.l2a": "Sentinel-2 L2A",
  "science.sentinel-2.l1c.v1": "Sentinel-2 L1C",
  "optical.sentinel-2.l1c": "Sentinel-2 L1C",
  "science.sentinel-3.standard.v1": "Sentinel-3",
  "optical.sentinel-3.standard": "Sentinel-3",
  "science.landsat.c2-l2.v1": "Landsat C2 L2",
  "optical.landsat.c2.l2": "Landsat C2 L2",
  "science.hls.l30.v1": "HLS L30",
  "science.hls.l30.v2": "HLS L30",
  "optical.hls.l30.v2": "HLS L30",
  "science.hls.s30.v1": "HLS S30",
  "science.hls.s30.v2": "HLS S30",
  "optical.hls.s30.v2": "HLS S30",
  "science.nasa.emit.l2a.v1": "NASA EMIT L2A",
  "nasa_emit": "NASA EMIT L2A",
  "science.dlr.enmap.l2a.v1": "DLR EnMAP L2A",
  "dlr_enmap": "DLR EnMAP L2A",
  "atmospheric.sentinel-5p.l2.v1": "Sentinel-5P TROPOMI",
  "sentinel_5p": "Sentinel-5P TROPOMI",
  "science.esa.worldcover.v1": "ESA WorldCover 10m",
  "esa_worldcover": "ESA WorldCover 10m",
  "science.jrc.gsw.v1": "JRC Global Surface Water",
  "jrc_gsw": "JRC Global Surface Water",
  "science.modis.ndvi.v1": "MODIS 250m NDVI",
  "modis_ndvi": "MODIS 250m NDVI",
  "science.nasa.black-marble.v1": "NASA Black Marble",
  "nasa_black_marble": "NASA Black Marble",
  "science.nasa.ecostress.l2t.v1": "NASA ECOSTRESS",
  "nasa_ecostress": "NASA ECOSTRESS",
  "science.overture.maps.v1": "Overture Maps",
  "overture_maps": "Overture Maps",
  "science.whu.clcd.v1": "WHU CLCD 30m",
  "whu_clcd": "WHU CLCD 30m",
  "science.tum.eot20.v1": "TUM EOT20",
  "tum_eot20": "TUM EOT20",
  "science.gebco.bathymetry-2026.v1": "GEBCO Bathymetry",
  "gebco_2026": "GEBCO Bathymetry",
  "science.noaa.etopo-2022.v1": "NOAA ETOPO 2022",
  "noaa_etopo_2022": "NOAA ETOPO 2022",
  "orbit.sentinel-1.poeorb.v1": "Sentinel-1 Precise Orbit",
  "orbit.copernicus.pod.v1": "Copernicus POD Orbit",
  "copernicus_pod_orbit": "Copernicus POD Orbit",
  "orbit.nisar.oe.v1": "NISAR Orbit Ephemeris",
  "nisar_orbit_oe": "NISAR Orbit Ephemeris",
  "atmospheric.era5.reanalysis.v1": "ERA5 Reanalysis",
  "open_meteo_era5": "ERA5 Reanalysis",
  "atmospheric.gacos.ztd.v1": "GACOS ZTD",
  "ionospheric.igs.gim.v1": "IGS/CODE GIM",
  "ionospheric_ionex": "IGS/CODE GIM",
  "science.noaa.aws-weather.v1": "NOAA AWS Weather",
  "noaa_aws_weather": "NOAA AWS Weather",
  "event.usgs.earthquake.v1": "USGS Earthquake",
  "usgs_earthquake": "USGS Earthquake",
  "geodesy.ngl.gnss.v1": "NGL GNSS",
  "ngl_gnss": "NGL GNSS",
  "science.gee.catalog.v1": "Google Earth Engine",
  "gee_catalog": "Google Earth Engine",
};

const READABLE_PROVIDER_NAMES: Readonly<Record<string, string>> = {
  "isro.nrsc.bhoonidhi": "ISRO Bhoonidhi",
  "asf.nasa.earthdata": "NASA ASF",
  "asf.catalog": "NASA ASF",
  asf: "NASA ASF",
  "copernicus.dataspace.odata": "Copernicus CDSE",
  "cdse.copernicus": "Copernicus CDSE",
  "microsoft.planetary-computer": "Planetary Computer",
  "planetary-computer": "Planetary Computer",
  "mpc.stac": "Planetary Computer",
  "usgs.eros.m2m": "USGS EarthExplorer",
  "earth-search.aws": "AWS Earth Search",
  "aws.earth-search": "AWS Earth Search",
  "google.earth-engine": "Google Earth Engine",
  "Google Earth Engine": "Google Earth Engine",
};

function cleanString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function isOpaqueUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
    value.trim(),
  );
}

function stripInternalEnvelope(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  const itemMarker = "::ITEM::";
  const itemIdx = trimmed.indexOf(itemMarker);
  if (trimmed.startsWith("PROVIDER::") && itemIdx >= 0) {
    return trimmed.slice(itemIdx + itemMarker.length).trim();
  }
  if (trimmed.startsWith("gee-composite::")) {
    return trimmed.slice("gee-composite::".length).trim();
  }
  if (trimmed.startsWith("gee-scene::")) {
    return trimmed.slice("gee-scene::".length).trim();
  }
  return trimmed;
}

function isUsefulPolarization(value: unknown): value is string {
  const s = cleanString(value);
  return s.length > 0 && s !== "-" && s !== "—";
}

function formatOrbitDirEn(value: unknown): string | null {
  const raw = cleanString(value).toUpperCase();
  if (!raw) return null;
  if (raw === "ASCENDING" || raw.startsWith("ASC")) return "Ascending";
  if (raw === "DESCENDING" || raw.startsWith("DESC")) return "Descending";
  return cleanString(value) || null;
}

function formatLookDirEn(value: unknown): string | null {
  const raw = cleanString(value).toUpperCase();
  if (!raw) return null;
  if (raw === "L" || raw === "LEFT") return "Left";
  if (raw === "R" || raw === "RIGHT") return "Right";
  return cleanString(value) || null;
}

function normalizeIsoParts(raw: string): {
  date: string;
  hm: string;
  hms: string;
  hasTime: boolean;
} | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const dateOnly = /^(\d{4}-\d{2}-\d{2})$/.exec(trimmed);
  if (dateOnly) {
    return { date: dateOnly[1], hm: "", hms: "", hasTime: false };
  }
  const iso = /^(\d{4}-\d{2}-\d{2})[T ](\d{2}:\d{2})(?::(\d{2}))?(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?$/i.exec(
    trimmed,
  );
  if (iso) {
    const sec = iso[3] ?? "00";
    return {
      date: iso[1],
      hm: iso[2],
      hms: `${iso[2]}:${sec}`,
      hasTime: true,
    };
  }
  return null;
}

/** Format timestamp for dense result rows, e.g. `2026-10-12 22:49 UTC` or `2026-10-12`. */
export function formatCompactUtc(iso?: string | null): string {
  const raw = cleanString(iso);
  if (!raw) return "";
  const parsed = normalizeIsoParts(raw);
  if (!parsed) return raw;
  return parsed.hasTime ? `${parsed.date} ${parsed.hm} UTC` : parsed.date;
}

/** Format full start/stop acquisition time in clean UTC, e.g. `2026-11-06 23:48:45 ~ 23:49:21 UTC`. */
export function formatFullUtc(start?: string | null, stop?: string | null): string {
  const s = cleanString(start);
  const e = cleanString(stop);
  if (!s) return "";
  const sp = normalizeIsoParts(s);
  if (!sp) return e && e !== s ? `${s} ~ ${e}` : s;
  const startFormatted = sp.hasTime ? `${sp.date} ${sp.hms} UTC` : sp.date;
  if (!e || e === s) return startFormatted;
  const ep = normalizeIsoParts(e);
  if (!ep) return `${startFormatted} ~ ${e}`;
  if (sp.hasTime && ep.hasTime && sp.date === ep.date) {
    if (sp.hms === ep.hms) return startFormatted;
    return `${sp.date} ${sp.hms} ~ ${ep.hms} UTC`;
  }
  const stopFormatted = ep.hasTime ? `${ep.date} ${ep.hms} UTC` : ep.date;
  return startFormatted === stopFormatted ? startFormatted : `${startFormatted} ~ ${stopFormatted}`;
}

function sceneNativeRecord(scene: SceneRow): Record<string, unknown> {
  if (scene.native_metadata && typeof scene.native_metadata === "object" && !Array.isArray(scene.native_metadata)) {
    return scene.native_metadata as Record<string, unknown>;
  }
  const productMeta = scene.sar_product?.native_metadata;
  if (productMeta && typeof productMeta === "object" && !Array.isArray(productMeta)) {
    return productMeta as Record<string, unknown>;
  }
  return {};
}

function scenePropsRecord(scene: SceneRow): Record<string, unknown> {
  const meta = sceneNativeRecord(scene);
  const props = meta.properties;
  return props && typeof props === "object" && !Array.isArray(props)
    ? (props as Record<string, unknown>)
    : {};
}

function readSceneField(scene: SceneRow, ...keys: string[]): unknown {
  const meta = sceneNativeRecord(scene);
  const props = scenePropsRecord(scene);
  for (const key of keys) {
    if (hasValue(props[key])) return props[key];
    if (hasValue(meta[key])) return meta[key];
  }
  return null;
}

function isBhoonidhiScene(scene: SceneRow): boolean {
  const product = scene.sar_product;
  if (product && isBhoonidhiSsarCatalogProduct(product)) return true;
  const logical = cleanString(readSceneField(scene, "logical_dataset"));
  return (
    cleanString(scene.provider) === "isro.nrsc.bhoonidhi" ||
    cleanString(scene.dataset) === "sar.nisar.s.ssar.rslc.v1" ||
    cleanString(scene.dataset) === "science.nisar.s.ssar.rslc.v1" ||
    logical === "sar.nisar.s.ssar.rslc.v1"
  );
}

/** Extract official human-readable product identifier without exposing internal store envelopes. */
export function resolveAuthoritativeProductId(scene: SceneRow): string {
  const product = scene.sar_product;
  const landsatPid = cleanString(readSceneField(scene, "landsat_product_id"));
  const producerGranule = cleanString(readSceneField(scene, "producer_granule_id"));
  const geeAsset = cleanString(readSceneField(scene, "gee_asset_id"));
  const isComposite = scene.scene_id.startsWith("gee-composite::");
  if (isComposite && scene.file_name?.trim()) {
    return scene.file_name.trim();
  }
  if (geeAsset && !isComposite) {
    return scene.file_name?.trim() || geeAsset;
  }
  if (landsatPid) return landsatPid;

  const isBhoonidhi = isBhoonidhiScene(scene);
  if (isBhoonidhi) {
    const candidates = [
      cleanString(product?.provider_product_id),
      cleanString(readSceneField(scene, "provider_product_id")),
      cleanString(readSceneField(scene, "ID", "id")),
      stripInternalEnvelope(cleanString(scene.scene_id)),
      cleanString(scene.file_name),
    ];
    for (const candidate of candidates) {
      if (candidate && !isOpaqueUuid(candidate)) return candidate;
    }
  }

  if (producerGranule && !isOpaqueUuid(producerGranule)) return producerGranule;

  const prodPid = cleanString(product?.provider_product_id);
  if (prodPid && !isOpaqueUuid(prodPid)) return prodPid;

  const candidates = [
    cleanString(scene.file_name),
    cleanString(product?.scene_id),
    stripInternalEnvelope(cleanString(scene.scene_id)),
    stripInternalEnvelope(cleanString(scene.product_id)),
    cleanString(readSceneField(scene, "ID", "id")),
    prodPid,
  ];
  for (const candidate of candidates) {
    if (candidate) return candidate;
  }
  return scene.scene_id;
}

/** Extract optional secondary identifier (e.g. Bhoonidhi Scene_Identifier or Landsat entity_id). */
export function resolveSceneSecondaryId(scene: SceneRow, primaryId: string): string | null {
  const candidates = [
    cleanString(readSceneField(scene, "Scene_Identifier", "scene_identifier")),
    cleanString(readSceneField(scene, "entity_id")),
    cleanString(scene.file_name),
    cleanString(scene.sar_product?.scene_id),
  ];
  for (const candidate of candidates) {
    if (candidate && candidate !== primaryId && !candidate.startsWith("PROVIDER::")) {
      return candidate;
    }
  }
  return null;
}

function localizeEarthquakeDepthInTitle(raw: string, targetLocale: Locale): string {
  if (!raw || targetLocale === "zh-CN") return raw;
  const depthRe = /\(深度\s*([0-9]+(?:\.[0-9]+)?)\s*km\)/g;
  if (targetLocale === "ja") {
    return raw.replace(depthRe, "（震源深さ $1 km）");
  }
  return raw.replace(depthRe, "(Depth $1 km)");
}

/** Resolve primary list-row display title across all datasets without exposing internal prefixes. */
export function formatSceneRowDisplayTitle(scene: SceneRow): string {
  const locale = getCurrentLocale();
  if (isBhoonidhiScene(scene)) {
    const authId = resolveAuthoritativeProductId(scene);
    if (authId && !authId.startsWith("PROVIDER::")) return localizeEarthquakeDepthInTitle(authId, locale);
  }
  const fileName = cleanString(scene.file_name);
  if (fileName) return localizeEarthquakeDepthInTitle(fileName, locale);
  return localizeEarthquakeDepthInTitle(resolveAuthoritativeProductId(scene), locale);
}

/** Map internal dataset and provider identifiers into clean source labels. */
export function resolveReadableSourceLabel(scene: SceneRow, targetLocale: Locale = "en"): string {
  const product = scene.sar_product;
  const meta = sceneNativeRecord(scene);
  const logical = cleanString(meta.logical_dataset);
  const datasetRaw = cleanString(scene.dataset) || cleanString(product?.dataset) || logical;
  const providerRaw = cleanString(scene.provider) || cleanString(product?.provider);

  const datasetName =
    READABLE_DATASET_NAMES[datasetRaw] ||
    READABLE_DATASET_NAMES[logical] ||
    (datasetRaw && !datasetRaw.includes(".") ? datasetRaw : "");
  let providerName = READABLE_PROVIDER_NAMES[providerRaw] || "";
  if (!providerName && providerRaw) {
    if (isDatasetDisplayNameMatch(providerRaw, scene)) {
      const dsId = resolveSceneDatasetId(scene);
      providerName = datasetName || !dsId ? "" : localizedSourceDisplayName(dsId, targetLocale);
    } else if (!providerRaw.includes(".") && providerRaw !== datasetName) {
      providerName = providerRaw;
    }
  }

  if (datasetName && providerName) {
    if (
      datasetName.toLowerCase().includes(providerName.toLowerCase()) ||
      providerName.toLowerCase().includes(datasetName.toLowerCase())
    ) {
      return datasetName;
    }
    return `${datasetName} (${providerName})`;
  }
  return datasetName || providerName || resolvePlatformLabel(scene, targetLocale) || "EOStage Catalog";
}

/** Classify any SceneRow into its canonical scientific display category. */
export function classifySceneCategory(
  scene: SceneRow,
  family?: string | null,
): SceneDisplayCategory {
  const ds = (
    cleanString(scene.dataset) ||
    cleanString(scene.sar_product?.dataset) ||
    cleanString(readSceneField(scene, "logical_dataset"))
  ).toLowerCase();
  const id = resolveAuthoritativeProductId(scene).toUpperCase();
  const meta = sceneNativeRecord(scene);

  if (
    scene.scene_id.startsWith("gee-composite::") ||
    scene.scene_id.startsWith("gee-scene::") ||
    ds.startsWith("science.gee.") ||
    ds === "gee_catalog"
  ) {
    return "gee";
  }
  if (
    ds.startsWith("orbit.") ||
    ds.includes("orbit") ||
    /_(POEORB|RESORB|POE_AX|FOE)_/.test(id)
  ) {
    return "orbit";
  }
  if (ds === "event.usgs.earthquake.v1" || ds === "usgs_earthquake") {
    return "event";
  }
  if (ds === "geodesy.ngl.gnss.v1" || ds === "ngl_gnss") {
    return "geodesy";
  }
  if (
    family === "optical" ||
    meta.product_kind === "optical_scene" ||
    ds.startsWith("optical.") ||
    [
      "science.sentinel-2.l2a.v1",
      "science.sentinel-2.l1c.v1",
      "science.landsat.c2-l2.v1",
      "science.hls.l30.v1",
      "science.hls.l30.v2",
      "science.hls.s30.v1",
      "science.hls.s30.v2",
      "science.sentinel-3.standard.v1",
      "science.nasa.emit.l2a.v1",
      "science.dlr.enmap.l2a.v1",
      "nasa_emit",
      "dlr_enmap",
    ].includes(ds)
  ) {
    return "optical";
  }
  if (
    ds.startsWith("atmospheric.") ||
    ds.startsWith("ionospheric.") ||
    ds.startsWith("auxiliary.") ||
    ds.startsWith("dem.") ||
    [
      "science.alos.palsar-mosaic.v1",
      "alos_palsar_mosaic",
      "science.esa.worldcover.v1",
      "esa_worldcover",
      "science.jrc.gsw.v1",
      "jrc_gsw",
      "science.modis.ndvi.v1",
      "modis_ndvi",
      "science.nasa.black-marble.v1",
      "nasa_black_marble",
      "science.nasa.ecostress.l2t.v1",
      "nasa_ecostress",
      "science.noaa.aws-weather.v1",
      "noaa_aws_weather",
      "science.overture.maps.v1",
      "overture_maps",
      "science.whu.clcd.v1",
      "whu_clcd",
      "science.tum.eot20.v1",
      "tum_eot20",
      "science.gebco.bathymetry-2026.v1",
      "gebco_2026",
      "science.noaa.etopo-2022.v1",
      "noaa_etopo_2022",
    ].includes(ds)
  ) {
    return "thematic";
  }
  return "sar";
}

function detectTierForMeta(scene: SceneRow): "validated" | "provisional" | null {
  const explicit = cleanString(readSceneField(scene, "release_tier", "tier")).toLowerCase();
  if (explicit === "validated" || explicit === "provisional") return explicit;
  const conceptId = cleanString(
    readSceneField(scene, "collection_concept_id", "native_collection_id", "collectionConceptId"),
  ).toUpperCase();
  if (VALIDATED_NISAR_COLLECTIONS.has(conceptId)) return "validated";
  if (PROVISIONAL_NISAR_COLLECTIONS.has(conceptId)) return "provisional";
  const shortName = cleanString(
    readSceneField(scene, "collection_short_name", "short_name", "shortName"),
  ).toUpperCase();
  if (shortName.includes("PROVISIONAL") || shortName.includes("BETA")) return "provisional";
  if (/^NISAR_L[123]_[A-Z0-9_]+_V\d+$/.test(shortName)) return "validated";
  return null;
}

function resolveSceneDatasetId(scene: SceneRow): string {
  return (
    cleanString(scene.dataset) ||
    cleanString(scene.sar_product?.dataset) ||
    cleanString(readSceneField(scene, "logical_dataset"))
  );
}

function isDatasetDisplayNameMatch(raw: string, scene: SceneRow): boolean {
  const ds = resolveSceneDatasetId(scene);
  if (!ds || !raw) return false;
  const knownNames = new Set([
    canonicalSourceDisplayName(ds),
    localizedSourceDisplayName(ds, "zh-CN"),
    localizedSourceDisplayName(ds, "en"),
    localizedSourceDisplayName(ds, "ja"),
  ]);
  return knownNames.has(raw);
}

function localizeDatasetMatchedLabel(raw: string, scene: SceneRow, targetLocale: Locale): string {
  const ds = resolveSceneDatasetId(scene);
  if (!ds) return raw;
  return isDatasetDisplayNameMatch(raw, scene) ? localizedSourceDisplayName(ds, targetLocale) : raw;
}

function resolvePlatformLabel(scene: SceneRow, targetLocale: Locale = getCurrentLocale()): string {
  const raw = cleanString(scene.platform) || cleanString(scene.sar_product?.platform);
  if (!raw || raw.includes(".")) return "";
  return localizeDatasetMatchedLabel(raw, scene, targetLocale);
}

function resolveSensorLabel(scene: SceneRow): string {
  const extra = scene as unknown as Record<string, unknown>;
  return (
    cleanString(extra.sensor) ||
    cleanString(scene.instrument) ||
    cleanString(scene.sar_product?.instrument) ||
    cleanString(readSceneField(scene, "Instrument", "instrument"))
  );
}

function resolveTrackNumber(scene: SceneRow): number | null {
  const product = scene.sar_product;
  for (const cand of [
    scene.relative_orbit,
    scene.path,
    product?.relative_orbit,
    product?.path,
    optionalFiniteNumber(readSceneField(scene, "relative_orbit", "Track", "track")),
  ]) {
    if (typeof cand === "number" && Number.isFinite(cand) && cand > 0) return cand;
  }
  return null;
}

function resolveFrameNumber(scene: SceneRow): number | null {
  const product = scene.sar_product;
  for (const cand of [
    scene.frame,
    product?.frame,
    optionalFiniteNumber(readSceneField(scene, "Frame", "frame")),
  ]) {
    if (typeof cand === "number" && Number.isFinite(cand) && cand >= 0) {
      if (cand === 0 && cleanString(scene.orbit_direction) === "" && !product) return null;
      return cand;
    }
  }
  return null;
}

function resolveTileOrWrsLabel(scene: SceneRow): { key: "Tile" | "WRS Path/Row"; value: string } | null {
  const product = scene.sar_product;
  if (product && (isSentinel2L2aCatalogProduct(product) || isSentinel2L1cCatalogProduct(product))) {
    const s2 = sentinel2Tile(product);
    if (s2) return { key: "Tile", value: s2 };
  }
  const wrsPath = cleanString(readSceneField(scene, "wrs_path", "landsat:wrs_path"));
  const wrsRow = cleanString(readSceneField(scene, "wrs_row", "landsat:wrs_row"));
  if (wrsPath && wrsRow) {
    return { key: "WRS Path/Row", value: `${wrsPath.padStart(3, "0")}/${wrsRow.padStart(3, "0")}` };
  }
  const mgrs = cleanString(readSceneField(scene, "mgrs_tile", "mgrs:tile", "tile_id", "grid_code"));
  if (mgrs) return { key: "Tile", value: mgrs };
  const name = resolveAuthoritativeProductId(scene);
  const hlsTile = /^HLS\.[LS]30\.(T\d{2}[A-Z]{3})\./i.exec(name)?.[1];
  if (hlsTile) return { key: "Tile", value: hlsTile.toUpperCase() };
  const s2Tile = /_(T\d{2}[A-Z]{3})_/i.exec(name)?.[1];
  if (s2Tile) return { key: "Tile", value: s2Tile.toUpperCase() };
  return null;
}

function resolveCenterLatLon(scene: SceneRow, bbox: ReturnType<typeof resolveSceneBbox>): { lon: number; lat: number } | null {
  const latVal = optionalFiniteNumber(
    readSceneField(scene, "SceneCenterLat", "centerLat", "centerLatitude") ??
      firstProperty(scene.sar_product ?? ({} as SarProduct), ["centerLat", "centerLatitude"]),
  );
  const lonVal = optionalFiniteNumber(
    readSceneField(scene, "SceneCenterLon", "centerLon", "centerLongitude") ??
      firstProperty(scene.sar_product ?? ({} as SarProduct), ["centerLon", "centerLongitude"]),
  );
  if (
    Number.isFinite(latVal) &&
    Number.isFinite(lonVal) &&
    latVal >= -90 &&
    latVal <= 90 &&
    lonVal >= -180 &&
    lonVal <= 180
  ) {
    return { lon: lonVal, lat: latVal };
  }
  if (bbox) {
    let lon = bbox.west > bbox.east ? (bbox.west + bbox.east + 360) / 2 : (bbox.west + bbox.east) / 2;
    if (lon > 180) lon -= 360;
    return { lon, lat: (bbox.south + bbox.north) / 2 };
  }
  return null;
}

function normalizeLevelOrType(scene: SceneRow): string | null {
  const candidate =
    cleanString(scene.sar_product?.processing_level) ||
    cleanString(scene.processing_level) ||
    cleanString(scene.product_type);
  if (!candidate || ["STANDARD", "GRID"].includes(candidate.toUpperCase())) return null;
  return candidate;
}

/** Build category-aware, noise-free second-line metadata fields for CatalogResultRow. */
export function buildSceneRowMetaDisplay(
  scene: SceneRow,
  family?: string | null,
): SceneRowMetaDisplay {
  const category = classifySceneCategory(scene, family);
  const product = scene.sar_product;
  const compactTime = formatCompactUtc(scene.acquisition_datetime);
  const sizeLabel = resolveSceneSizeLabel(scene);
  const rawPol = cleanString(scene.polarization) || cleanString(product?.polarization);
  const polarization = isUsefulPolarization(rawPol) ? rawPol : null;
  const cloud = resolveSceneCloudCover(scene);
  const cloudText = cloud !== null ? t("mapWorkbench.row.cloudCover", { value: Math.round(cloud * 10) / 10 }) : null;
  const tileInfo = resolveTileOrWrsLabel(scene);
  const tileOrWrs = tileInfo
    ? tileInfo.key === "WRS Path/Row"
      ? `WRS ${tileInfo.value}`
      : `Tile ${tileInfo.value}`
    : null;
  const platformRaw = resolvePlatformLabel(scene);
  const platform = platformRaw && platformRaw !== resolveReadableSourceLabel(scene) ? platformRaw : null;
  const levelOrType = normalizeLevelOrType(scene);
  const tier = detectTierForMeta(scene);

  const dirUpper = (cleanString(scene.orbit_direction) || cleanString(product?.flight_direction)).toUpperCase();
  const orbitDotKind: "asc" | "desc" | null =
    dirUpper === "ASCENDING" || dirUpper.startsWith("ASC")
      ? "asc"
      : dirUpper === "DESCENDING" || dirUpper.startsWith("DESC")
        ? "desc"
        : null;
  const orbitDirText = orbitDotKind === "asc"
    ? t("mapWorkbench.row.ascending")
    : orbitDotKind === "desc"
      ? t("mapWorkbench.row.descending")
      : "";

  const trackVal = resolveTrackNumber(scene);
  const isNisar =
    cleanString(scene.dataset).toLowerCase().includes("nisar") ||
    cleanString(product?.dataset).toLowerCase().includes("nisar") ||
    resolveAuthoritativeProductId(scene).toUpperCase().startsWith("NISAR_");
  const orbitTrackText = trackVal !== null ? `${isNisar ? "Track" : "Path"} ${trackVal}` : null;
  const frameVal = resolveFrameNumber(scene);
  const frameText = frameVal !== null ? `Frame ${frameVal}` : null;

  let scaleOrSpanText: string | null = null;
  if (category === "gee") {
    const matched = optionalFiniteNumber(readSceneField(scene, "matched_scenes"));
    const total = optionalFiniteNumber(readSceneField(scene, "total_scenes"));
    const scaleM = optionalFiniteNumber(readSceneField(scene, "native_scale_m"));
    if (Number.isFinite(matched) && Number.isFinite(total)) {
      scaleOrSpanText = t("mapWorkbench.sceneSummary.geeValidScenes", { matched: matched!, total: total! });
    } else if (Number.isFinite(scaleM)) {
      scaleOrSpanText = `${scaleM}m`;
    }
  } else if (category === "geodesy") {
    const span = cleanString(readSceneField(scene, "time_span"));
    if (span) scaleOrSpanText = span;
  } else if (category === "thematic") {
    const res = cleanString(readSceneField(scene, "resolution", "spatial_resolution"));
    if (res) scaleOrSpanText = res;
  }

  let placeOrStation: string | null = null;
  if (category === "event") {
    const depth = optionalFiniteNumber(readSceneField(scene, "depth_km", "depth"));
    const place = cleanString(readSceneField(scene, "place"));
    const parts = [
      Number.isFinite(depth) ? t("mapWorkbench.sceneSummary.eventDepthKm", { depth: depth!.toFixed(1) }) : "",
      place,
    ].filter(Boolean);
    if (parts.length > 0) placeOrStation = parts.join(" · ");
  } else if (category === "geodesy") {
    const station = cleanString(readSceneField(scene, "station_id", "site", "code"));
    if (station) placeOrStation = t("mapWorkbench.sceneSummary.geodesyStation", { station });
  }

  return {
    category,
    compactTime,
    orbitDirText,
    orbitDotKind,
    orbitTrackText,
    frameText,
    polarization,
    levelOrType,
    platform,
    tileOrWrs,
    cloudText,
    scaleOrSpanText,
    placeOrStation,
    sizeLabel,
    tier,
  };
}

/** Build 6–9 high-signal scientific summary rows for result row `i` hover popover (Chinese concise labels). */
export function buildSceneSummaryRows(scene: SceneRow): SceneSummaryRow[] {
  const rows: SceneSummaryRow[] = [];
  const product = scene.sar_product;
  const category = classifySceneCategory(scene);
  const locale = getCurrentLocale();
  const productId = localizeEarthquakeDepthInTitle(resolveAuthoritativeProductId(scene), locale);
  const secondaryIdRaw = resolveSceneSecondaryId(scene, resolveAuthoritativeProductId(scene));
  const secondaryId = secondaryIdRaw ? localizeEarthquakeDepthInTitle(secondaryIdRaw, locale) : null;
  const sourceLabel = resolveReadableSourceLabel(scene, locale);

  if (productId) rows.push({ label: t("mapWorkbench.sceneSummary.productId"), value: productId });
  if (secondaryId) rows.push({ label: t("mapWorkbench.sceneSummary.sceneNumber"), value: secondaryId });
  if (sourceLabel) rows.push({ label: t("mapWorkbench.sceneSummary.dataSource"), value: sourceLabel });

  const startUtc = cleanString(scene.acquisition_datetime) || cleanString(product?.acquisition_start);
  const stopUtc = cleanString(product?.acquisition_stop) || cleanString(readSceneField(scene, "end_date"));
  const fullUtc = formatFullUtc(startUtc, stopUtc);
  if (fullUtc) rows.push({ label: t("mapWorkbench.sceneSummary.acquisitionTime"), value: fullUtc });

  const levelStr = normalizeLevelOrType(scene) ?? "";
  const modeStr = cleanString(scene.beam_mode) || cleanString(product?.beam_mode);
  const rawPol = cleanString(scene.polarization) || cleanString(product?.polarization);
  const polStr = isUsefulPolarization(rawPol) ? rawPol : "";
  const platformStr = resolvePlatformLabel(scene);
  const sensorStr = resolveSensorLabel(scene);
  const specParts = [
    category === "sar" || category === "optical" ? levelStr : "",
    category === "sar" && modeStr ? t("mapWorkbench.sceneSummary.modePrefix", { mode: modeStr }) : "",
    polStr ? t("mapWorkbench.sceneSummary.polPrefix", { pol: polStr }) : "",
  ].filter(Boolean);
  if (specParts.length > 0) {
    rows.push({ label: t("mapWorkbench.sceneSummary.productParams"), value: specParts.join(" · ") });
  } else if (platformStr || sensorStr || levelStr) {
    const platParts = Array.from(new Set([platformStr, sensorStr, levelStr].filter(Boolean)));
    if (platParts.length > 0) rows.push({ label: t("mapWorkbench.sceneSummary.platformLevel"), value: platParts.join(" · ") });
  }

  const metaDisplay = buildSceneRowMetaDisplay(scene, category);
  if (category === "sar") {
    const orbitParts = [metaDisplay.orbitDirText, metaDisplay.orbitTrackText, metaDisplay.frameText].filter(Boolean);
    if (orbitParts.length > 0) rows.push({ label: t("mapWorkbench.sceneSummary.orbitInfo"), value: orbitParts.join(" · ") });
  } else if (category === "optical" && metaDisplay.tileOrWrs) {
    rows.push({ label: t("mapWorkbench.sceneSummary.tileOrWrs"), value: metaDisplay.tileOrWrs });
  }

  const cloud = resolveSceneCloudCover(scene);
  if (cloud !== null) {
    rows.push({ label: t("mapWorkbench.sceneSummary.cloudCover"), value: `${cloud.toFixed(1)}%` });
  } else if (metaDisplay.scaleOrSpanText) {
    rows.push({
      label: category === "gee" ? t("mapWorkbench.sceneSummary.coverageStats") : t("mapWorkbench.sceneSummary.scaleOrSpan"),
      value: metaDisplay.scaleOrSpanText,
    });
  }

  const bbox = resolveSceneBbox(scene);
  const center = resolveCenterLatLon(scene, bbox);
  if (center) {
    rows.push({
      label: t("mapWorkbench.sceneSummary.centerPosition"),
      value: `${formatSignedLon(center.lon)}, ${formatSignedLat(center.lat)}`,
    });
  }
  if (bbox) {
    rows.push({
      label: t("mapWorkbench.sceneSummary.wgs84Bounds"),
      value: `${formatSignedLon(bbox.west)} ~ ${formatSignedLon(bbox.east)} · ${formatSignedLat(bbox.south)} ~ ${formatSignedLat(bbox.north)}`,
    });
  }

  const sizeLabel = resolveSceneSizeLabel(scene);
  if (sizeLabel) {
    rows.push({ label: t("agent.sceneDrawer.size"), value: sizeLabel });
  }

  return rows;
}

/** Build clean, standard English key-value metadata block for clipboard copy when clicking `i`. */
export function buildSceneFullCopyText(scene: SceneRow): string {
  const lines: string[] = [];
  const product = scene.sar_product;
  const category = classifySceneCategory(scene);
  const rawProductId = resolveAuthoritativeProductId(scene);
  const productId = localizeEarthquakeDepthInTitle(rawProductId, "en");
  const secondaryIdRaw = resolveSceneSecondaryId(scene, rawProductId);
  const secondaryId = secondaryIdRaw ? localizeEarthquakeDepthInTitle(secondaryIdRaw, "en") : null;
  const sourceLabel = resolveReadableSourceLabel(scene, "en");

  append(lines, "Product ID", productId);
  if (secondaryId) append(lines, "Scene ID", secondaryId);
  append(lines, "Source", sourceLabel);

  const platformStr = resolvePlatformLabel(scene, "en");
  const sensorStr = resolveSensorLabel(scene);
  const platSensor = Array.from(new Set([platformStr, sensorStr].filter(Boolean))).join(" / ");
  if (platSensor && platSensor !== sourceLabel) {
    append(lines, "Platform / Sensor", platSensor);
  }

  const levelStr = normalizeLevelOrType(scene);
  const modeStr = cleanString(scene.beam_mode) || cleanString(product?.beam_mode);
  const rawPol = cleanString(scene.polarization) || cleanString(product?.polarization);
  const polStr = isUsefulPolarization(rawPol) ? rawPol : null;
  const cloud = resolveSceneCloudCover(scene);

  if (category === "sar") {
    append(lines, "Product Level", levelStr);
    if (modeStr && !["GRID", "STANDARD"].includes(modeStr.toUpperCase())) {
      append(lines, "Mode", modeStr);
    }
    append(lines, "Polarization", polStr);
    const tier = detectTierForMeta(scene);
    if (tier) append(lines, "Release Tier", tier === "validated" ? "Validated" : "Provisional");

    append(lines, "Orbit Direction", formatOrbitDirEn(scene.orbit_direction || product?.flight_direction));
    const trackVal = resolveTrackNumber(scene);
    if (trackVal !== null) append(lines, "Track", trackVal);
    const frameVal = resolveFrameNumber(scene);
    if (frameVal !== null) append(lines, "Frame", frameVal);
    const absOrbit = scene.absolute_orbit ?? product?.absolute_orbit ?? optionalFiniteNumber(readSceneField(scene, "Imaging_Orbit", "absolute_orbit"));
    if (typeof absOrbit === "number" && Number.isFinite(absOrbit) && absOrbit > 0) {
      append(lines, "Absolute Orbit", absOrbit);
    }
    append(lines, "Look Direction", formatLookDirEn(product?.look_direction ?? readSceneField(scene, "look_direction", "Look_dir")));
    const incAngle = optionalFiniteNumber(readSceneField(scene, "IncidenceAngle", "centerIncidenceAngle", "incidenceAngle"));
    if (Number.isFinite(incAngle)) append(lines, "Incidence Angle", `${incAngle.toFixed(2)} deg`);
  } else if (category === "optical") {
    append(lines, "Product Level", levelStr);
    if (cloud !== null) append(lines, "Cloud Cover", `${cloud.toFixed(1)}%`);
    const tileInfo = resolveTileOrWrsLabel(scene);
    if (tileInfo) append(lines, tileInfo.key, tileInfo.value);
  } else if (category === "gee") {
    append(lines, "Asset ID", readSceneField(scene, "gee_asset_id"));
    const scaleM = optionalFiniteNumber(readSceneField(scene, "native_scale_m"));
    if (Number.isFinite(scaleM)) append(lines, "Scale", `${scaleM} m`);
    const matched = optionalFiniteNumber(readSceneField(scene, "matched_scenes"));
    const total = optionalFiniteNumber(readSceneField(scene, "total_scenes"));
    if (Number.isFinite(matched) && Number.isFinite(total)) {
      append(lines, "Matched Scenes", `${matched} / ${total}`);
    }
    if (cloud !== null) append(lines, "Cloud Cover", `${cloud.toFixed(1)}%`);
  } else if (category === "orbit") {
    append(lines, "Orbit Type", levelStr || cleanString(readSceneField(scene, "orbit_type")));
  } else if (category === "event") {
    const mag = optionalFiniteNumber(readSceneField(scene, "magnitude", "mag"));
    if (Number.isFinite(mag)) append(lines, "Magnitude", `Mw ${mag.toFixed(1)}`);
    else append(lines, "Magnitude", levelStr);
    const depth = optionalFiniteNumber(readSceneField(scene, "depth_km", "depth"));
    if (Number.isFinite(depth)) append(lines, "Depth", `${depth.toFixed(1)} km`);
    append(lines, "Place", readSceneField(scene, "place"));
  } else if (category === "geodesy") {
    append(lines, "Station", readSceneField(scene, "station_id", "site", "code"));
    append(lines, "Product Type", levelStr);
    append(lines, "Time Span", readSceneField(scene, "time_span"));
  } else {
    append(lines, "Product Type", levelStr);
    append(lines, "Polarization", polStr);
    append(lines, "Resolution", readSceneField(scene, "resolution", "spatial_resolution"));
    if (cloud !== null) append(lines, "Cloud Cover", `${cloud.toFixed(1)}%`);
  }

  const startUtc = cleanString(scene.acquisition_datetime) || cleanString(product?.acquisition_start);
  const stopUtc = cleanString(product?.acquisition_stop) || cleanString(readSceneField(scene, "end_date"));
  const fullUtc = formatFullUtc(startUtc, stopUtc);
  if (fullUtc) append(lines, "Acquisition UTC", fullUtc);

  const bbox = resolveSceneBbox(scene);
  const center = resolveCenterLatLon(scene, bbox);
  if (center) {
    append(lines, "Center", `${formatSignedLon(center.lon)}, ${formatSignedLat(center.lat)}`);
  }
  if (bbox) {
    append(
      lines,
      "BBox (WGS84)",
      `${formatSignedLon(bbox.west)} ~ ${formatSignedLon(bbox.east)}, ${formatSignedLat(bbox.south)} ~ ${formatSignedLat(bbox.north)}`,
    );
  }

  const sizeLabel = resolveSceneSizeLabel(scene);
  if (sizeLabel) append(lines, "File Size", sizeLabel);

  const checksum = typedChecksum(product?.checksum);
  if (checksum) append(lines, "Checksum", `${checksum[0].toUpperCase()}:${checksum[1]}`);

  return lines.join("\n");
}
