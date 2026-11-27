import "../test-utils/setup-dom.ts";
import assert from "node:assert/strict";
import test from "node:test";

import {
  categorizeSentinel1Source,
  clearPreciseOrbitPreference,
  getDefaultPreciseOrbit,
  getOrbitPreferenceSourceKey,
  getPreciseOrbitPreference,
  isSentinel1OrbitApplicable,
  setPreciseOrbitPreference,
} from "./preciseOrbitPreferences.ts";

test.beforeEach(() => {
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.clear();
  }
});

test.afterEach(() => {
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.clear();
  }
});

test("isSentinel1OrbitApplicable detects Sentinel-1 sources and products", () => {
  // Sentinel-1 SLC
  assert.equal(isSentinel1OrbitApplicable("science.sentinel-1.standard.v1", "SLC"), true);
  assert.equal(isSentinel1OrbitApplicable("asf.sentinel1", "SLC"), true);
  assert.equal(isSentinel1OrbitApplicable("sentinel1_slc"), true);
  assert.equal(isSentinel1OrbitApplicable("Sentinel-1 SLC"), true);
  assert.equal(isSentinel1OrbitApplicable("Sentinel-1", "SLC"), true);

  // Sentinel-1 Burst
  assert.equal(isSentinel1OrbitApplicable("science.sentinel-1.standard.v1", "BURST"), true);
  assert.equal(isSentinel1OrbitApplicable("science.sentinel-1.standard.v1", "SLC-BURST"), true);
  assert.equal(isSentinel1OrbitApplicable("sentinel1_burst"), true);
  assert.equal(isSentinel1OrbitApplicable("Sentinel-1 Burst"), true);

  // Sentinel-1 GRD
  assert.equal(isSentinel1OrbitApplicable("science.sentinel-1.standard.v1", "GRD"), true);
  assert.equal(isSentinel1OrbitApplicable("asf.sentinel1", "GRD"), true);
  assert.equal(isSentinel1OrbitApplicable("sentinel1_grd"), true);
  assert.equal(isSentinel1OrbitApplicable("Sentinel-1 GRD"), true);

  // Optical and non-Sentinel-1 sources: NOT applicable
  assert.equal(isSentinel1OrbitApplicable("science.sentinel-2.l2a.v1", "S2MSI2A"), false);
  assert.equal(isSentinel1OrbitApplicable("science.landsat.c2-l2.v1"), false);
  assert.equal(isSentinel1OrbitApplicable("copernicus.dataspace.odata"), false);
  assert.equal(isSentinel1OrbitApplicable("alos2"), false);
  assert.equal(isSentinel1OrbitApplicable("nisar"), false);
  assert.equal(isSentinel1OrbitApplicable(null, null), false);
});

test("categorizeSentinel1Source correctly classifies categories", () => {
  assert.equal(categorizeSentinel1Source("science.sentinel-1.standard.v1", "SLC"), "slc");
  assert.equal(categorizeSentinel1Source("asf.sentinel1", "SLC"), "slc");
  assert.equal(categorizeSentinel1Source("sentinel1_slc"), "slc");

  assert.equal(categorizeSentinel1Source("science.sentinel-1.standard.v1", "BURST"), "burst");
  assert.equal(categorizeSentinel1Source("science.sentinel-1.standard.v1", "SLC-BURST"), "burst");
  assert.equal(categorizeSentinel1Source("sentinel1_burst"), "burst");

  assert.equal(categorizeSentinel1Source("science.sentinel-1.standard.v1", "GRD"), "grd");
  assert.equal(categorizeSentinel1Source("sentinel1_grd"), "grd");

  assert.equal(categorizeSentinel1Source("science.sentinel-2.l2a.v1"), null);
  assert.equal(categorizeSentinel1Source("landsat"), null);
});

test("getDefaultPreciseOrbit returns true for SLC and Burst, false for GRD and others", () => {
  // SLC -> true
  assert.equal(getDefaultPreciseOrbit("science.sentinel-1.standard.v1", "SLC"), true);
  assert.equal(getDefaultPreciseOrbit("asf.sentinel1", "SLC"), true);
  assert.equal(getDefaultPreciseOrbit("sentinel1_slc"), true);

  // Burst -> true
  assert.equal(getDefaultPreciseOrbit("science.sentinel-1.standard.v1", "BURST"), true);
  assert.equal(getDefaultPreciseOrbit("sentinel1_burst"), true);

  // GRD -> false
  assert.equal(getDefaultPreciseOrbit("science.sentinel-1.standard.v1", "GRD"), false);
  assert.equal(getDefaultPreciseOrbit("asf.sentinel1", "GRD"), false);
  assert.equal(getDefaultPreciseOrbit("sentinel1_grd"), false);

  // Non-Sentinel-1 -> false
  assert.equal(getDefaultPreciseOrbit("science.sentinel-2.l2a.v1"), false);
  assert.equal(getDefaultPreciseOrbit("science.landsat.c2-l2.v1"), false);
});

test("getOrbitPreferenceSourceKey provides clean persistent keys", () => {
  assert.equal(getOrbitPreferenceSourceKey("science.sentinel-1.standard.v1", "SLC"), "sentinel1_slc");
  assert.equal(getOrbitPreferenceSourceKey("asf.sentinel1", "SLC"), "sentinel1_slc");
  assert.equal(getOrbitPreferenceSourceKey("sentinel1_slc"), "sentinel1_slc");
  assert.equal(getOrbitPreferenceSourceKey("sentinel1_burst"), "sentinel1_burst");
  assert.equal(getOrbitPreferenceSourceKey("science.sentinel-1.standard.v1", "GRD"), "sentinel1_grd");
});

test("preference persistence remembers toggled choices per source", () => {
  // 1. Initial state without storage: defaults to true for SLC, false for GRD
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC"), true);
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "GRD"), false);

  // 2. User unchecks SLC -> persists false
  setPreciseOrbitPreference("science.sentinel-1.standard.v1", false, "SLC");
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC"), false);
  // GRD is unaffected
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "GRD"), false);

  // 3. User checks GRD -> persists true
  setPreciseOrbitPreference("science.sentinel-1.standard.v1", true, "GRD");
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "GRD"), true);
  // SLC remains false
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC"), false);

  // 4. Burst is independently true
  assert.equal(getPreciseOrbitPreference("sentinel1_burst"), true);
  setPreciseOrbitPreference("sentinel1_burst", false);
  assert.equal(getPreciseOrbitPreference("sentinel1_burst"), false);

  // 5. Clear resets back to default
  clearPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC");
  assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC"), true);
});

test("storage failure fails closed gracefully to default policy", () => {
  const originalGetItem = window.localStorage.getItem;
  try {
    window.localStorage.getItem = () => {
      throw new Error("Storage unavailable / SecurityError");
    };
    // Must not throw, should return policy default
    assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "SLC"), true);
    assert.equal(getPreciseOrbitPreference("science.sentinel-1.standard.v1", "GRD"), false);
  } finally {
    window.localStorage.getItem = originalGetItem;
  }
});
