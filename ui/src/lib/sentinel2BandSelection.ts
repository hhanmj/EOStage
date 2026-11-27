import { t } from "../i18n/index.ts";

export type Sentinel2DeliveryForm = "safe_archive" | "analysis_bands";
export type Sentinel2BandPreset = "true_color" | "vegetation" | "all_10m" | "all_reflectance" | "custom";

export type Sentinel2BandDefinition = Readonly<{
  key: string;
  label: string;
  description: string;
  resolutionM: 10 | 20 | 60;
}>;

export const SENTINEL2_REFLECTANCE_BANDS: readonly Sentinel2BandDefinition[] = Object.freeze([
  { key: "B01", get label() { return t("shared.sentinel2Bands.B01"); }, description: "Coastal aerosol", resolutionM: 60 },
  { key: "B02", get label() { return t("shared.sentinel2Bands.B02"); }, description: "Blue", resolutionM: 10 },
  { key: "B03", get label() { return t("shared.sentinel2Bands.B03"); }, description: "Green", resolutionM: 10 },
  { key: "B04", get label() { return t("shared.sentinel2Bands.B04"); }, description: "Red", resolutionM: 10 },
  { key: "B05", get label() { return t("shared.sentinel2Bands.B05"); }, description: "Vegetation red edge 1", resolutionM: 20 },
  { key: "B06", get label() { return t("shared.sentinel2Bands.B06"); }, description: "Vegetation red edge 2", resolutionM: 20 },
  { key: "B07", get label() { return t("shared.sentinel2Bands.B07"); }, description: "Vegetation red edge 3", resolutionM: 20 },
  { key: "B08", get label() { return t("shared.sentinel2Bands.B08"); }, description: "Near infrared", resolutionM: 10 },
  { key: "B8A", get label() { return t("shared.sentinel2Bands.B8A"); }, description: "Narrow near infrared", resolutionM: 20 },
  { key: "B09", get label() { return t("shared.sentinel2Bands.B09"); }, description: "Water vapour", resolutionM: 60 },
  { key: "B11", get label() { return t("shared.sentinel2Bands.B11"); }, description: "SWIR 1", resolutionM: 20 },
  { key: "B12", get label() { return t("shared.sentinel2Bands.B12"); }, description: "SWIR 2", resolutionM: 20 },
]);

const SENTINEL2_BAND_KEYS = new Set(SENTINEL2_REFLECTANCE_BANDS.map((band) => band.key));

export function sentinel2BandDefinition(key: string): Sentinel2BandDefinition | null {
  return SENTINEL2_REFLECTANCE_BANDS.find((band) => band.key === key) ?? null;
}

export function sentinel2BandKeysForPreset(preset: Sentinel2BandPreset): string[] {
  switch (preset) {
    case "true_color":
      return ["B02", "B03", "B04"];
    case "vegetation":
      return ["B04", "B08"];
    case "all_10m":
      return ["B02", "B03", "B04", "B08"];
    case "all_reflectance":
      return SENTINEL2_REFLECTANCE_BANDS.map((band) => band.key);
    case "custom":
      return [];
  }
}

export function sentinel2BandPresetLabel(preset: Sentinel2BandPreset): string {
  switch (preset) {
    case "true_color":
      return t("shared.sentinel2Bands.presets.true_color");
    case "vegetation":
      return t("shared.sentinel2Bands.presets.vegetation");
    case "all_10m":
      return t("shared.sentinel2Bands.presets.all_10m");
    case "all_reflectance":
      return t("shared.sentinel2Bands.presets.all_reflectance");
    case "custom":
      return t("shared.sentinel2Bands.presets.custom");
  }
}

export function normalizeSentinel2BandKeys(keys: readonly string[]): string[] {
  const selected = new Set(keys.filter((key) => SENTINEL2_BAND_KEYS.has(key)));
  return SENTINEL2_REFLECTANCE_BANDS
    .map((band) => band.key)
    .filter((key) => selected.has(key));
}

export function validSentinel2BandKeys(keys: readonly string[]): boolean {
  return keys.length > 0
    && keys.length <= SENTINEL2_REFLECTANCE_BANDS.length
    && new Set(keys).size === keys.length
    && keys.every((key) => SENTINEL2_BAND_KEYS.has(key));
}

export const SENTINEL2_L1C_BANDS: readonly Sentinel2BandDefinition[] = Object.freeze([
  { key: "B01", get label() { return t("shared.sentinel2Bands.B01"); }, description: "Coastal aerosol (.jp2)", resolutionM: 60 },
  { key: "B02", get label() { return t("shared.sentinel2Bands.B02"); }, description: "Blue (.jp2)", resolutionM: 10 },
  { key: "B03", get label() { return t("shared.sentinel2Bands.B03"); }, description: "Green (.jp2)", resolutionM: 10 },
  { key: "B04", get label() { return t("shared.sentinel2Bands.B04"); }, description: "Red (.jp2)", resolutionM: 10 },
  { key: "B05", get label() { return t("shared.sentinel2Bands.B05"); }, description: "Vegetation red edge 1 (.jp2)", resolutionM: 20 },
  { key: "B06", get label() { return t("shared.sentinel2Bands.B06"); }, description: "Vegetation red edge 2 (.jp2)", resolutionM: 20 },
  { key: "B07", get label() { return t("shared.sentinel2Bands.B07"); }, description: "Vegetation red edge 3 (.jp2)", resolutionM: 20 },
  { key: "B08", get label() { return t("shared.sentinel2Bands.B08"); }, description: "Near infrared (.jp2)", resolutionM: 10 },
  { key: "B8A", get label() { return t("shared.sentinel2Bands.B8A"); }, description: "Narrow near infrared (.jp2)", resolutionM: 20 },
  { key: "B09", get label() { return t("shared.sentinel2Bands.B09"); }, description: "Water vapour (.jp2)", resolutionM: 60 },
  { key: "B10", label: "B10 卷云波段 (1375 nm)", description: "Cirrus SWIR (.jp2)", resolutionM: 60 },
  { key: "B11", get label() { return t("shared.sentinel2Bands.B11"); }, description: "SWIR 1 (.jp2)", resolutionM: 20 },
  { key: "B12", get label() { return t("shared.sentinel2Bands.B12"); }, description: "SWIR 2 (.jp2)", resolutionM: 20 },
]);

const SENTINEL2_L1C_BAND_KEYS = new Set(SENTINEL2_L1C_BANDS.map((band) => band.key));

export function sentinel2L1cBandKeysForPreset(preset: Sentinel2BandPreset): string[] {
  switch (preset) {
    case "true_color":
      return ["B04", "B03", "B02"];
    case "vegetation":
      return ["B08", "B04", "B03", "B02"];
    case "all_10m":
      return ["B02", "B03", "B04", "B08"];
    case "all_reflectance":
      return SENTINEL2_L1C_BANDS.map((band) => band.key);
    case "custom":
      return [];
  }
}

export function normalizeSentinel2L1cBandKeys(keys: readonly string[]): string[] {
  const selected = new Set(keys.filter((key) => SENTINEL2_L1C_BAND_KEYS.has(key)));
  return SENTINEL2_L1C_BANDS
    .map((band) => band.key)
    .filter((key) => selected.has(key));
}

export function validSentinel2L1cBandKeys(keys: readonly string[]): boolean {
  return keys.length > 0
    && keys.length <= SENTINEL2_L1C_BANDS.length
    && new Set(keys).size === keys.length
    && keys.every((key) => SENTINEL2_L1C_BAND_KEYS.has(key));
}

export type HlsBandDefinition = Readonly<{
  key: string;
  label: string;
  description: string;
  resolutionM: 30;
}>;

export type HlsBandPreset = "true_color" | "false_color" | "swir" | "all_bands" | "custom";

export const HLS_L30_BAND_OPTIONS: readonly HlsBandDefinition[] = Object.freeze([
  { key: "B01", label: "B01 海岸/气溶胶", description: "Coastal aerosol (OLI)", resolutionM: 30 },
  { key: "B02", label: "B02 蓝波段", description: "Blue", resolutionM: 30 },
  { key: "B03", label: "B03 绿波段", description: "Green", resolutionM: 30 },
  { key: "B04", label: "B04 红波段", description: "Red", resolutionM: 30 },
  { key: "B05", label: "B05 近红外 NIR", description: "NIR", resolutionM: 30 },
  { key: "B06", label: "B06 短波红外 SWIR 1", description: "SWIR 1", resolutionM: 30 },
  { key: "B07", label: "B07 短波红外 SWIR 2", description: "SWIR 2", resolutionM: 30 },
  { key: "B09", label: "B09 卷云 Cirrus", description: "Cirrus", resolutionM: 30 },
  { key: "B10", label: "B10 热红外 TIR 1", description: "TIRS 1", resolutionM: 30 },
  { key: "B11", label: "B11 热红外 TIR 2", description: "TIRS 2", resolutionM: 30 },
  { key: "Fmask", label: "Fmask 云雪水体掩膜", description: "Quality bitmask", resolutionM: 30 },
]);

export const HLS_S30_BAND_OPTIONS: readonly HlsBandDefinition[] = Object.freeze([
  { key: "B01", label: "B01 海岸/气溶胶", description: "Coastal aerosol (MSI)", resolutionM: 30 },
  { key: "B02", label: "B02 蓝波段", description: "Blue", resolutionM: 30 },
  { key: "B03", label: "B03 绿波段", description: "Green", resolutionM: 30 },
  { key: "B04", label: "B04 红波段", description: "Red", resolutionM: 30 },
  { key: "B05", label: "B05 红边 Red Edge 1", description: "Red Edge 1", resolutionM: 30 },
  { key: "B06", label: "B06 红边 Red Edge 2", description: "Red Edge 2", resolutionM: 30 },
  { key: "B07", label: "B07 红边 Red Edge 3", description: "Red Edge 3", resolutionM: 30 },
  { key: "B08", label: "B08 宽波段 NIR", description: "NIR Broad", resolutionM: 30 },
  { key: "B8A", label: "B8A 窄波段 NIR", description: "NIR Narrow", resolutionM: 30 },
  { key: "B09", label: "B09 水汽", description: "Water vapour", resolutionM: 30 },
  { key: "B10", label: "B10 卷云", description: "Cirrus", resolutionM: 30 },
  { key: "B11", label: "B11 短波红外 SWIR 1", description: "SWIR 1", resolutionM: 30 },
  { key: "B12", label: "B12 短波红外 SWIR 2", description: "SWIR 2", resolutionM: 30 },
  { key: "Fmask", label: "Fmask 云雪水体掩膜", description: "Quality bitmask", resolutionM: 30 },
]);

export function hlsBandOptionsForProduct(product: "L30" | "S30"): readonly HlsBandDefinition[] {
  return product === "L30" ? HLS_L30_BAND_OPTIONS : HLS_S30_BAND_OPTIONS;
}

export function hlsBandKeysForPreset(product: "L30" | "S30", preset: HlsBandPreset): string[] {
  if (product === "L30") {
    switch (preset) {
      case "true_color": return ["B04", "B03", "B02"];
      case "false_color": return ["B05", "B04", "B03"];
      case "swir": return ["B07", "B06", "B04"];
      case "all_bands": return HLS_L30_BAND_OPTIONS.map((b) => b.key);
      case "custom": return [];
    }
  }
  switch (preset) {
    case "true_color": return ["B04", "B03", "B02"];
    case "false_color": return ["B8A", "B04", "B03"];
    case "swir": return ["B12", "B11", "B04"];
    case "all_bands": return HLS_S30_BAND_OPTIONS.map((b) => b.key);
    case "custom": return [];
  }
}

export function normalizeHlsBandKeys(product: "L30" | "S30", keys: readonly string[]): string[] {
  const options = hlsBandOptionsForProduct(product);
  const validSet = new Set(options.map((b) => b.key));
  const selected = new Set(keys.filter((k) => validSet.has(k)));
  return options.map((b) => b.key).filter((k) => selected.has(k));
}
