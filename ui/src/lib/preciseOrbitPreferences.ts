import { getStorage } from "./webStorage.ts";

export type Sentinel1OrbitCategory = "slc" | "burst" | "grd";

const PRECISE_ORBIT_STORAGE_PREFIX = "eostage.precise_orbit.";

function normalizeText(value?: string | null): string {
  return typeof value === "string" ? value.trim().toLowerCase() : "";
}

/**
 * Categorize a Sentinel-1 source and product type into "slc" | "burst" | "grd",
 * or null if companion orbit does not apply (e.g. Optical, Landsat, NISAR, non-S1).
 */
export function categorizeSentinel1Source(
  source?: string | null,
  productType?: string | null,
): Sentinel1OrbitCategory | null {
  const normSource = normalizeText(source);
  const normProduct = normalizeText(productType);

  // If source explicitly denotes a non-Sentinel-1 platform/resource, it is NOT applicable.
  if (
    normSource.includes("sentinel-2") ||
    normSource.includes("sentinel2") ||
    normSource.includes("landsat") ||
    normSource.includes("copernicus.dataspace.odata") ||
    normSource.includes("alos") ||
    normSource.includes("nisar") ||
    normSource.includes("hls.")
  ) {
    return null;
  }

  // Check for Burst indicators
  if (
    normProduct === "burst" ||
    normProduct === "slc-burst" ||
    normSource.includes("burst")
  ) {
    return "burst";
  }

  // Check for GRD indicators
  if (normProduct === "grd" || normSource.includes("grd")) {
    return "grd";
  }

  // Check for Sentinel-1 SLC indicators
  const isS1Source =
    normSource.includes("sentinel-1") ||
    normSource.includes("sentinel1") ||
    normSource === "asf.sentinel1" ||
    normSource === "asf.catalog" ||
    normSource === "science.sentinel-1.standard.v1";

  if (normProduct === "slc" || normSource.includes("slc")) {
    return "slc";
  }

  if (isS1Source) {
    // Default Sentinel-1 source mode without explicit non-SLC product is SLC
    return "slc";
  }

  return null;
}

/**
 * Determine whether companion precise orbit applies to the source and product type.
 * Returns true for Sentinel-1 SLC, Burst, and GRD. Returns false for all other sources.
 */
export function isSentinel1OrbitApplicable(
  source?: string | null,
  productType?: string | null,
): boolean {
  return categorizeSentinel1Source(source, productType) !== null;
}

/**
 * Resolve the canonical storage key for a source / dataset.
 */
export function getOrbitPreferenceSourceKey(
  source?: string | null,
  productType?: string | null,
): string {
  const category = categorizeSentinel1Source(source, productType);
  if (category === "burst") return "sentinel1_burst";
  if (category === "grd") return "sentinel1_grd";
  if (category === "slc") return "sentinel1_slc";
  return normalizeText(source) || "unknown";
}

/**
 * Default policy value for include_precise_orbit:
 * - SLC -> true
 * - Burst -> true
 * - GRD -> false
 * - non-Sentinel-1 -> false
 */
export function getDefaultPreciseOrbit(
  source?: string | null,
  productType?: string | null,
): boolean {
  const category = categorizeSentinel1Source(source, productType);
  if (category === "slc" || category === "burst") return true;
  return false;
}

/**
 * Retrieve the active include_precise_orbit preference for the source.
 * Fails closed to policy default if storage is unavailable.
 */
export function getPreciseOrbitPreference(
  source?: string | null,
  productType?: string | null,
): boolean {
  const defaultVal = getDefaultPreciseOrbit(source, productType);
  try {
    const storage = getStorage();
    if (!storage) return defaultVal;

    const canonicalKey = getOrbitPreferenceSourceKey(source, productType);
    if (canonicalKey) {
      const canonical = storage.getItem(`${PRECISE_ORBIT_STORAGE_PREFIX}${canonicalKey}`);
      if (canonical === "true") return true;
      if (canonical === "false") return false;
    }
  } catch {
    // Fail closed on storage error
  }
  return defaultVal;
}

/**
 * Persist user preference for include_precise_orbit for the source.
 * Fails closed (ignores errors) if storage is unavailable.
 */
export function setPreciseOrbitPreference(
  source: string | null | undefined,
  enabled: boolean,
  productType?: string | null,
): void {
  try {
    const storage = getStorage();
    if (!storage) return;

    const value = enabled ? "true" : "false";
    const canonicalKey = getOrbitPreferenceSourceKey(source, productType);
    if (canonicalKey) {
      storage.setItem(`${PRECISE_ORBIT_STORAGE_PREFIX}${canonicalKey}`, value);
    }
  } catch {
    // Fail closed on storage error
  }
}

/**
 * Clear stored preference for testing / reset.
 */
export function clearPreciseOrbitPreference(
  source?: string | null,
  productType?: string | null,
): void {
  try {
    const storage = getStorage();
    if (!storage) return;

    const canonicalKey = getOrbitPreferenceSourceKey(source, productType);
    if (canonicalKey) {
      storage.removeItem(`${PRECISE_ORBIT_STORAGE_PREFIX}${canonicalKey}`);
    }
  } catch {
    // Fail closed
  }
}
