import { t } from "../i18n/index.ts";
import type { DesktopLandsatAssetDetail } from "./desktopShellBridge.ts";

export type LandsatContentSelection =
  | "all"
  | "surface_reflectance"
  | "surface_temperature"
  | "custom";

const SURFACE_REFLECTANCE_KEYS = new Set([
  "coastal",
  "blue",
  "green",
  "red",
  "nir08",
  "swir16",
  "swir22",
  "qa_pixel",
  "qa_radsat",
  "qa_aerosol",
  "ang",
  "mtl.txt",
  "mtl.xml",
]);

const SURFACE_TEMPERATURE_KEYS = new Set([
  "lwir11",
  "drad",
  "emis",
  "emsd",
  "trad",
  "urad",
  "atran",
  "cdist",
  "qa",
  "qa_pixel",
  "qa_radsat",
  "ang",
  "mtl.txt",
  "mtl.xml",
]);

export function landsatAssetKeysForPreset(
  assets: readonly DesktopLandsatAssetDetail[],
  selection: Exclude<LandsatContentSelection, "custom">,
): string[] {
  if (selection === "all") return assets.map((asset) => asset.asset_key);
  if (selection === "surface_temperature" && !landsatHasSurfaceTemperature(assets)) return [];
  const allowed = selection === "surface_reflectance"
    ? SURFACE_REFLECTANCE_KEYS
    : SURFACE_TEMPERATURE_KEYS;
  return assets
    .filter((asset) => allowed.has(asset.asset_key))
    .map((asset) => asset.asset_key);
}

export function landsatSelectionSizeBytes(
  assets: readonly DesktopLandsatAssetDetail[],
  selectedAssetKeys: readonly string[],
): number {
  const selected = new Set(selectedAssetKeys);
  return assets.reduce(
    (sum, asset) => selected.has(asset.asset_key) ? sum + asset.size_bytes : sum,
    0,
  );
}

export function landsatHasSurfaceTemperature(
  assets: readonly DesktopLandsatAssetDetail[],
): boolean {
  return assets.some((asset) => asset.asset_key === "lwir11");
}

export function landsatContentSelectionLabel(selection: LandsatContentSelection): string {
  if (selection === "surface_reflectance") return t("shared.landsatSelection.surfaceReflectance");
  if (selection === "surface_temperature") return t("shared.landsatSelection.surfaceTemperature");
  if (selection === "custom") return t("shared.landsatSelection.custom");
  return t("shared.landsatSelection.fullProduct");
}
