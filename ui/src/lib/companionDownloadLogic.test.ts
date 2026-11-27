/**
 * companionDownloadLogic.test.ts
 * 配套下载纯逻辑单元测试
 */

import assert from "node:assert/strict";
import test from "node:test";

if (typeof globalThis.localStorage === "undefined") {
  const memStore = new Map<string, string>();
  (globalThis as any).localStorage = {
    getItem: (k: string) => memStore.get(k) ?? null,
    setItem: (k: string, v: string) => memStore.set(k, String(v)),
    removeItem: (k: string) => memStore.delete(k),
    clear: () => memStore.clear(),
  };
}

import {
  isSentinel1Resource,
  getDefaultPreciseOrbit,
  getSavedCompanionOrbitPreference,
  setSavedCompanionOrbitPreference,
  calculateOrbitMatch,
  estimateDemTiles,
  estimateGacosRequests,
  resolveAuxiliaryExtent,
  scenesCoverageBbox,
} from "./companionDownloadLogic.ts";
import type { SceneRow, Bbox } from "./bridge.ts";

test("companionDownloadLogic test suite", async (suite) => {
  await suite.test("1. isSentinel1Resource and default precise orbit policy", () => {
    assert.strictEqual(isSentinel1Resource("science.sentinel-1.standard.v1"), true);
    assert.strictEqual(isSentinel1Resource("science.sentinel-1.slc-burst.v1"), true);
    assert.strictEqual(isSentinel1Resource("science.sentinel-2.l2a.v1"), false);
    assert.strictEqual(isSentinel1Resource("science.aria.s1-gunw.v1"), false);
    assert.strictEqual(isSentinel1Resource("science.opera.disp-s1.v1"), false);
    assert.strictEqual(isSentinel1Resource("science.comet.licsar.v1"), false);

    // SLC / Burst default true, GRD default false
    assert.strictEqual(getDefaultPreciseOrbit("science.sentinel-1.standard.v1", "SLC"), true);
    assert.strictEqual(getDefaultPreciseOrbit("science.sentinel-1.standard.v1", "GRD"), false);
    assert.strictEqual(getDefaultPreciseOrbit("science.sentinel-2.l2a.v1", "L2A"), false);
  });

  await suite.test("2. preference persistence for companion orbit", () => {
    localStorage.clear();
    const sourceId = "science.sentinel-1.standard.v1";
    assert.strictEqual(getSavedCompanionOrbitPreference(sourceId, "SLC"), true);

    // Toggle to false
    setSavedCompanionOrbitPreference(sourceId, false);
    assert.strictEqual(getSavedCompanionOrbitPreference(sourceId, "SLC"), false);

    // Toggle back to true
    setSavedCompanionOrbitPreference(sourceId, true);
    assert.strictEqual(getSavedCompanionOrbitPreference(sourceId, "SLC"), true);
  });

  await suite.test("3. calculateOrbitMatch partitions scenes by 20-day publication rule", () => {
    const fixedNow = new Date("2026-11-05T00:00:00Z").getTime();

    const mockScenes: SceneRow[] = [
      {
        scene_id: "s1-old-1",
        acquisition_date: "2026-10-01T10:00:00Z", // > 20 days ago -> matched
      } as SceneRow,
      {
        scene_id: "s1-old-2",
        acquisition_date: "2026-10-10T10:00:00Z", // > 20 days ago -> matched
      } as SceneRow,
      {
        scene_id: "s1-recent-1",
        acquisition_date: "2026-10-30T10:00:00Z", // < 20 days ago -> pending
      } as SceneRow,
    ];

    const result = calculateOrbitMatch(mockScenes, "science.sentinel-1.standard.v1", fixedNow);
    assert.strictEqual(result.totalScenes, 3);
    assert.strictEqual(result.matchedCount, 2);
    assert.strictEqual(result.pendingCount, 1);
  });

  await suite.test("4. estimateDemTiles calculates 1x1 degree grid tiles", () => {
    // 2x3 degrees -> 6 tiles
    const bbox: Bbox = { west: 114.2, south: 30.1, east: 116.8, north: 32.5, crs: "EPSG:4326" };
    const estimate = estimateDemTiles(bbox);
    assert.strictEqual(estimate.estimatedTiles, 9); // west floor 114, east ceil 117 (3 lon) * south floor 30, north ceil 33 (3 lat) = 9
  });

  await suite.test("5. estimateGacosRequests follows 20-date batching and 10x10 degree spatial splitting", () => {
    // Case A: 3 scenes from 2 unique dates, within 2x2 degrees
    const scenesA: SceneRow[] = [
      { scene_id: "s1", acquisition_date: "2026-10-01" } as SceneRow,
      { scene_id: "s2", acquisition_date: "2026-10-01" } as SceneRow,
      { scene_id: "s3", acquisition_date: "2026-10-12" } as SceneRow,
    ];
    const bboxA: Bbox = { west: 110, south: 30, east: 112, north: 32, crs: "EPSG:4326" };
    const estA = estimateGacosRequests(scenesA, bboxA);
    assert.strictEqual(estA.uniqueDateCount, 2);
    assert.strictEqual(estA.dateBatches, 1);
    assert.strictEqual(estA.spatialBlocks, 1);
    assert.strictEqual(estA.requestCount, 1);

    // Case B: 25 unique dates -> 2 date batches
    const scenesB: SceneRow[] = Array.from({ length: 25 }, (_, i) => ({
      scene_id: `s-${i}`,
      acquisition_date: `2026-0${Math.floor(i / 28) + 1}-${(i % 28) + 1}`,
    })) as SceneRow[];
    const estB = estimateGacosRequests(scenesB, bboxA);
    assert.strictEqual(estB.uniqueDateCount, 25);
    assert.strictEqual(estB.dateBatches, 2);
    assert.strictEqual(estB.requestCount, 2);

    // Case C: Wide extent 15°x15° -> 4 spatial blocks * 1 date batch = 4 requests
    const bboxWide: Bbox = { west: 100, south: 20, east: 115, north: 35, crs: "EPSG:4326" };
    const estC = estimateGacosRequests(scenesA, bboxWide);
    assert.strictEqual(estC.spatialBlocks, 4);
    assert.strictEqual(estC.requestCount, 4);
  });

  await suite.test("6. resolveAuxiliaryExtent handles study_area, scene_coverage, and redraw_on_map", () => {
    const studyArea: Bbox = { west: 100, south: 20, east: 105, north: 25, crs: "EPSG:4326" };
    const scenes: SceneRow[] = [
      { scene_id: "1", footprint_bbox: { west: 102, south: 22, east: 104, north: 24, crs: "EPSG:4326" } } as SceneRow,
    ];
    const customBbox: Bbox = { west: 101, south: 21, east: 103, north: 23, crs: "EPSG:4326" };

    // study_area mode
    const extentStudy = resolveAuxiliaryExtent("study_area", studyArea, scenes);
    assert.deepStrictEqual(extentStudy, studyArea);

    // scene_coverage mode
    const extentScene = resolveAuxiliaryExtent("scene_coverage", studyArea, scenes);
    assert.deepStrictEqual(extentScene, scenes[0].footprint_bbox);

    // redraw_on_map mode
    const extentRedraw = resolveAuxiliaryExtent("redraw_on_map", studyArea, scenes, customBbox);
    assert.deepStrictEqual(extentRedraw, customBbox);
  });
});
