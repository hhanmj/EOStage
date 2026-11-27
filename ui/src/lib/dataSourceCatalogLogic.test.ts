/**
 * dataSourceCatalogLogic.test.ts
 * 数据源目录纯逻辑单元测试
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
  buildCatalogItems,
  CATALOG_METADATA_REGISTRY,
  canonicalSourceDisplayName,
  filterCatalogItems,
  computeCategoryCounts,
  getNextSelectableIndex,
  setAccountVerificationFailed,
  getRecentSourceIds,
  recordRecentSourceId,
  clearRecentSources,
  type CatalogItem,
} from "./dataSourceCatalogLogic.ts";
import type { SourceDescriptor } from "../shell/api/types.ts";

const mockDescriptors: SourceDescriptor[] = [
  {
    resource_id: "science.sentinel-1.standard.v1",
    group_id: "sentinel1",
    display_name_zh: "Sentinel-1",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 1000,
    accounts: ["asf.earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.sentinel-2.l2a.v1",
    group_id: "sentinel2",
    display_name_zh: "Sentinel-2 MSI",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "copernicus.s2.v1",
    filter_schema_revision: 1,
    result_cap: 500,
    accounts: ["copernicus.cdse"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  },
  {
    resource_id: "science.envisat.reserved.v1",
    group_id: "envisat",
    display_name_zh: "ENVISAT ASAR",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 200,
    accounts: ["esa"],
    network_default: "direct",
    availability: "unavailable",
    availability_reason_zh: "接入中",
  },
];

test("dataSourceCatalogLogic test suite", async (suite) => {
  await suite.test("1. buildCatalogItems merges descriptors and sets correct statuses", () => {
    const items = buildCatalogItems(mockDescriptors);
    assert.ok(items.length > 10, "Should contain registered catalog items");

    const s1 = items.find((i) => i.resourceId === "science.sentinel-1.standard.v1");
    assert.ok(s1);
    assert.strictEqual(s1.name, "Sentinel-1");
    assert.strictEqual(s1.status, "available");
    assert.strictEqual(s1.disabled, false);
    assert.strictEqual(s1.bandTag, "C");

    const s2 = items.find((i) => i.resourceId === "science.sentinel-2.l2a.v1");
    assert.ok(s2);
    assert.strictEqual(s2.name, "Sentinel-2");
    assert.strictEqual(s2.category, "primary-optical");
    assert.strictEqual(s2.status, "available");
    assert.strictEqual(
      items.find((i) => i.resourceId === "science.sentinel-2.l1c.v1"),
      undefined,
      "Sentinel-2 L1C sublevel should merge into unified Sentinel-2 catalog entry"
    );
    assert.strictEqual(canonicalSourceDisplayName("science.sentinel-2.l2a.v1"), "Sentinel-2 L2A");
    assert.strictEqual(canonicalSourceDisplayName("science.sentinel-2.l1c.v1"), "Sentinel-2 L1C");

    const envisat = items.find((i) => i.resourceId === "science.envisat.reserved.v1");
    assert.strictEqual(envisat, undefined, "Retired ENVISAT should be excluded from catalog");
    assert.strictEqual(items.find((i) => i.resourceId === "dem.user-local.v1"), undefined);

    const nisarL = items.find((i) => i.resourceId === "science.nisar.l.rslc.provisional.v1");
    const nisarS = items.find((i) => i.resourceId === "science.nisar.s.ssar.rslc.v1");
    assert.strictEqual(nisarL?.supportsOnlineCropping, true);
    assert.strictEqual(nisarS?.supportsOnlineCropping, true);

    for (const cogOpticalId of [
      "science.sentinel-2.l2a.v1",
      "science.landsat.c2-l2.v1",
      "science.hls.l30.v2",
      "science.hls.s30.v2",
    ]) {
      assert.strictEqual(
        items.find((i) => i.resourceId === cogOpticalId)?.supportsOnlineCropping,
        true,
        `${cogOpticalId} should expose supportsOnlineCropping badge`
      );
    }
    assert.strictEqual(
      CATALOG_METADATA_REGISTRY["science.sentinel-2.l1c.v1"]?.supportsOnlineCropping,
      true,
      "science.sentinel-2.l1c.v1 registry entry should expose supportsOnlineCropping"
    );

    const worldcover = items.find((i) => i.resourceId === "science.esa.worldcover.v1");
    assert.strictEqual(worldcover?.era, "2020–2021");
    assert.ok(worldcover?.timeRange.includes("v100") && worldcover?.timeRange.includes("v200"));

    const itemsWithBhoonidhiReady = buildCatalogItems(mockDescriptors, { bhoonidhi: "logged_in" });
    assert.strictEqual(
      itemsWithBhoonidhiReady.find((i) => i.resourceId === "science.nisar.s.ssar.rslc.v1")?.status,
      "available"
    );

    setAccountVerificationFailed("bhoonidhi", true);
    const itemsWithBhoonidhiFailed = buildCatalogItems(mockDescriptors, { bhoonidhi: "login_required" });
    assert.strictEqual(
      itemsWithBhoonidhiFailed.find((i) => i.resourceId === "science.nisar.s.ssar.rslc.v1")?.status,
      "account_required"
    );
    setAccountVerificationFailed("bhoonidhi", false);

    // B9: unconfigured GEE resolves to search_only under live account statuses; configured GEE resolves to available
    const itemsGeeMissing = buildCatalogItems(mockDescriptors, { gee: "credentials_missing" });
    assert.strictEqual(
      itemsGeeMissing.find((i) => i.resourceId === "science.gee.catalog.v1")?.status,
      "search_only"
    );
    // Partial accountStatuses ({ gee }) must NOT mark unrelated sources (Sentinel-2 / Landsat) as missing credentials
    const s2WithOnlyGeeKey = itemsGeeMissing.find((i) => i.resourceId === "science.sentinel-2.l2a.v1");
    const landsatWithOnlyGeeKey = itemsGeeMissing.find((i) => i.resourceId === "science.landsat.c2-l2.v1");
    assert.strictEqual(s2WithOnlyGeeKey?.status, "available");
    assert.strictEqual(landsatWithOnlyGeeKey?.status, "available");
    assert.ok(!s2WithOnlyGeeKey?.accountText.includes("待配置凭据"));
    assert.ok(!landsatWithOnlyGeeKey?.accountText.includes("待配置凭据"));

    // Saved CDSE + USGS M2M credentials (login_required) resolve to available without false missing-credentials suffix
    const itemsCdseUsgsReady = buildCatalogItems(mockDescriptors, {
      cdse: "login_required",
      usgs_m2m: "login_required",
    });
    const s2Ready = itemsCdseUsgsReady.find((i) => i.resourceId === "science.sentinel-2.l2a.v1");
    const landsatReady = itemsCdseUsgsReady.find((i) => i.resourceId === "science.landsat.c2-l2.v1");
    assert.strictEqual(s2Ready?.status, "available");
    assert.strictEqual(s2Ready?.accountText, "CDSE 账号 / MPC · Earth Search 免账号");
    assert.strictEqual(landsatReady?.status, "available");
    assert.strictEqual(landsatReady?.accountText, "MPC 免账号 / USGS 账号");

    // Explicit missing CDSE credentials resolves Sentinel-2 to account_required
    const itemsCdseMissing = buildCatalogItems(mockDescriptors, { cdse: "credentials_missing" });
    const s2Missing = itemsCdseMissing.find((i) => i.resourceId === "science.sentinel-2.l2a.v1");
    assert.strictEqual(s2Missing?.status, "account_required");
    assert.strictEqual(s2Missing?.accountText, "CDSE 账号 / MPC · Earth Search 免账号（待配置凭据）");

    const itemsGeeConfigured = buildCatalogItems(mockDescriptors, { gee: "logged_in" });
    assert.strictEqual(
      itemsGeeConfigured.find((i) => i.resourceId === "science.gee.catalog.v1")?.status,
      "available"
    );
  });

  await suite.test("2. filterCatalogItems by category correctly", () => {
    const items = buildCatalogItems(mockDescriptors);

    // Primary SAR
    const sarItems = filterCatalogItems(items, { category: "primary-sar" });
    assert.ok(sarItems.length > 0);
    assert.ok(sarItems.every((i) => i.subcategories.includes("primary-sar")));
    assert.ok(sarItems.some((i) => i.resourceId === "science.sentinel-1.standard.v1"));
    assert.ok(!sarItems.some((i) => i.resourceId === "science.sentinel-2.l2a.v1"));

    // Primary Optical
    const opticalItems = filterCatalogItems(items, { category: "primary-optical" });
    assert.ok(opticalItems.length > 0);
    assert.ok(opticalItems.every((i) => i.subcategories.includes("primary-optical")));
    assert.ok(opticalItems.some((i) => i.resourceId === "science.sentinel-2.l2a.v1"));

    // Auxiliary Orbit
    const orbitItems = filterCatalogItems(items, { category: "auxiliary-orbit" });
    assert.ok(orbitItems.some((i) => i.resourceId === "orbit.sentinel-1.poeorb.v1"));

    // Auxiliary Atmospheric
    const atmosItems = filterCatalogItems(items, { category: "auxiliary-atmospheric" });
    assert.ok(atmosItems.some((i) => i.resourceId === "atmospheric.gacos.ztd.v1"));
  });

  await suite.test("3. filterCatalogItems by search query (case-insensitive)", () => {
    const items = buildCatalogItems(mockDescriptors);

    // Search "sentinel"
    const resultSentinel = filterCatalogItems(items, { category: "primary-all", query: "sentinel" });
    assert.ok(resultSentinel.length >= 2);
    assert.ok(resultSentinel.some((i) => i.resourceId === "science.sentinel-1.standard.v1"));
    assert.ok(resultSentinel.some((i) => i.resourceId === "science.sentinel-2.l2a.v1"));

    // Search by band "L 波段"
    const resultLBand = filterCatalogItems(items, { category: "primary-all", query: "l 波段" });
    assert.ok(resultLBand.some((i) => i.resourceId === "science.nisar.l.rslc.provisional.v1"));

    // Non-existent search
    const emptyResult = filterCatalogItems(items, { category: "primary-all", query: "xyz123nonexistent" });
    assert.strictEqual(emptyResult.length, 0);
  });

  await suite.test("4. filterCatalogItems by band and account tags", () => {
    const items = buildCatalogItems(mockDescriptors);

    // Filter by C band
    const cBandItems = filterCatalogItems(items, { category: "primary-sar", band: "C" });
    assert.ok(cBandItems.every((i) => i.bandTag === "C"));
    assert.ok(cBandItems.some((i) => i.resourceId === "science.sentinel-1.standard.v1"));

    // Filter by free account
    const freeItems = filterCatalogItems(items, { category: "primary-dem", account: "free" });
    assert.ok(freeItems.every((i) => i.accountRequirement === "free"));
  });

  await suite.test("5. recent sources persistence and order management", () => {
    clearRecentSources();
    assert.deepStrictEqual(getRecentSourceIds(), []);

    recordRecentSourceId("science.sentinel-1.standard.v1");
    recordRecentSourceId("science.sentinel-2.l2a.v1");
    assert.deepStrictEqual(getRecentSourceIds(), [
      "science.sentinel-2.l2a.v1",
      "science.sentinel-1.standard.v1",
    ]);

    // Re-recording sentinel-1 moves it to top
    recordRecentSourceId("science.sentinel-1.standard.v1");
    assert.deepStrictEqual(getRecentSourceIds(), [
      "science.sentinel-1.standard.v1",
      "science.sentinel-2.l2a.v1",
    ]);

    // Filtering by "recent" category
    const items = buildCatalogItems(mockDescriptors);
    const recentItems = filterCatalogItems(items, {
      category: "recent",
      recentIds: getRecentSourceIds(),
    });
    assert.strictEqual(recentItems.length, 2);

    clearRecentSources();
  });

  await suite.test("6. computeCategoryCounts counts correctly", () => {
    const items = buildCatalogItems(mockDescriptors);
    const counts = computeCategoryCounts(items, [
      "science.sentinel-1.standard.v1",
      "science.sentinel-2.l2a.v1",
    ]);

    assert.strictEqual(counts.recent, 2);
    assert.ok(counts["primary-all"] >= 5);
    assert.ok(counts["primary-sar"] >= 2);
    assert.ok(counts["primary-optical"] >= 1);
    assert.ok(counts["auxiliary-orbit"] >= 1);
    assert.ok(counts["auxiliary-atmospheric"] >= 1);
  });

  await suite.test("7. getNextSelectableIndex skips disabled items and wraps", () => {
    const mockItems: CatalogItem[] = [
      { resourceId: "item-0", disabled: false } as CatalogItem,
      { resourceId: "item-1", disabled: true } as CatalogItem, // disabled
      { resourceId: "item-2", disabled: false } as CatalogItem,
      { resourceId: "item-3", disabled: true } as CatalogItem, // disabled
      { resourceId: "item-4", disabled: false } as CatalogItem,
    ];

    // From -1 down -> 0
    assert.strictEqual(getNextSelectableIndex(mockItems, -1, "down"), 0);

    // From 0 down -> 2 (skips 1)
    assert.strictEqual(getNextSelectableIndex(mockItems, 0, "down"), 2);

    // From 2 down -> 4 (skips 3)
    assert.strictEqual(getNextSelectableIndex(mockItems, 2, "down"), 4);

    // From 4 down -> 0 (wrap around)
    assert.strictEqual(getNextSelectableIndex(mockItems, 4, "down"), 0);

    // From 0 up -> 4 (wrap around)
    assert.strictEqual(getNextSelectableIndex(mockItems, 0, "up"), 4);

    // From 4 up -> 2 (skips 3)
    assert.strictEqual(getNextSelectableIndex(mockItems, 4, "up"), 2);
  });

  await suite.test("8. GACOS & Sentinel-1 精密轨道已提升至 primary-all 主清单且保留辅助分类", () => {
    const items = buildCatalogItems(mockDescriptors);
    const primaryAll = filterCatalogItems(items, { category: "primary-all" });
    const auxOrbit = filterCatalogItems(items, { category: "auxiliary-orbit" });
    const auxAtmos = filterCatalogItems(items, { category: "auxiliary-atmospheric" });

    assert.ok(
      primaryAll.some((i) => i.resourceId === "orbit.sentinel-1.poeorb.v1"),
      "精密轨道应出现在全部主数据分类 (primary-all)"
    );
    assert.ok(
      primaryAll.some((i) => i.resourceId === "atmospheric.gacos.ztd.v1"),
      "GACOS 大气延迟改正应出现在全部主数据分类 (primary-all)"
    );
    assert.ok(
      auxOrbit.some((i) => i.resourceId === "orbit.sentinel-1.poeorb.v1"),
      "精密轨道仍保留在 auxiliary-orbit 辅助分类"
    );
    assert.ok(
      auxAtmos.some((i) => i.resourceId === "atmospheric.gacos.ztd.v1"),
      "GACOS 仍保留在 auxiliary-atmospheric 辅助分类"
    );

    assert.strictEqual(
      canonicalSourceDisplayName("orbit.sentinel-1.poeorb.v1"),
      "Sentinel-1 精密轨道 (POEORB/RESORB)"
    );
    assert.strictEqual(
      canonicalSourceDisplayName("atmospheric.gacos.ztd.v1"),
      "GACOS 大气延迟改正"
    );
  });

  await suite.test("9. Step 5A taxonomy audit: orbit-only auxiliary-orbit, auxiliary-geodesy, primary-thematic, and zero OlmoEarth", () => {
    const items = buildCatalogItems(mockDescriptors);
    const orbitIds = filterCatalogItems(items, { category: "auxiliary-orbit" })
      .map((i) => i.resourceId)
      .sort();
    assert.deepStrictEqual(orbitIds, [
      "orbit.copernicus.pod.v1",
      "orbit.nisar.oe.v1",
      "orbit.sentinel-1.poeorb.v1",
    ]);

    const geodesyIds = filterCatalogItems(items, { category: "auxiliary-geodesy" })
      .map((i) => i.resourceId)
      .sort();
    assert.deepStrictEqual(geodesyIds, [
      "event.usgs.earthquake.v1",
      "geodesy.ngl.gnss.v1",
    ]);

    const thematicIds = filterCatalogItems(items, { category: "primary-thematic" })
      .map((i) => i.resourceId)
      .sort();
    // Includes newly onboarded public thematic products: NASA Black Marble (VIIRS nighttime lights) and NASA ECOSTRESS (LST/ET)
    assert.deepStrictEqual(thematicIds, [
      "atmospheric.sentinel-5p.l2.v1",
      "auxiliary.xyz-tiles.v1",
      "science.esa.worldcover.v1",
      "science.gebco.bathymetry-2026.v1",
      "science.jrc.gsw.v1",
      "science.modis.ndvi.v1",
      "science.nasa.black-marble.v1",
      "science.nasa.ecostress.l2t.v1",
      "science.noaa.etopo-2022.v1",
      "science.overture.maps.v1",
      "science.tum.eot20.v1",
      "science.whu.clcd.v1",
    ]);

    const opticalIds = filterCatalogItems(items, { category: "primary-optical" }).map(
      (i) => i.resourceId
    );
    assert.strictEqual(opticalIds.includes("science.esa.worldcover.v1"), false);
    assert.strictEqual(opticalIds.includes("auxiliary.xyz-tiles.v1"), false);
    assert.strictEqual(opticalIds.includes("science.nasa.emit.l2a.v1"), true);
    assert.strictEqual(opticalIds.includes("science.dlr.enmap.l2a.v1"), true);

    const enmapItem = items.find((i) => i.resourceId === "science.dlr.enmap.l2a.v1");
    assert.ok(enmapItem);
    assert.strictEqual(enmapItem.status, "search_only");
    assert.strictEqual(enmapItem.accountRequirement, "free");

    const freeIds = filterCatalogItems(items, { category: "primary-all", account: "free" }).map(
      (i) => i.resourceId
    );
    assert.strictEqual(freeIds.includes("dem.fabdem.v1"), true);
    assert.strictEqual(freeIds.includes("science.hls.l30.v2"), true);
    assert.strictEqual(freeIds.includes("science.hls.s30.v2"), true);
    assert.strictEqual(canonicalSourceDisplayName("science.landsat.c2-l2.v1"), "Landsat 4-9 C2-L2");

    const registryText = JSON.stringify(CATALOG_METADATA_REGISTRY);
    assert.strictEqual(registryText.includes("OlmoEarth"), false);
    assert.strictEqual(
      canonicalSourceDisplayName("orbit.nisar.oe.v1"),
      "NISAR 精密/标称轨道历书 (POE/MOE/NOE/FOE)"
    );
    assert.strictEqual(
      canonicalSourceDisplayName("orbit.copernicus.pod.v1"),
      "Sentinel-2 / Sentinel-3 精密轨道 (POD POEORB)"
    );
    assert.strictEqual(
      canonicalSourceDisplayName("science.nasa.black-marble.v1"),
      "NASA Black Marble 全球夜光 (VIIRS)"
    );
    assert.strictEqual(
      canonicalSourceDisplayName("science.dlr.enmap.l2a.v1"),
      "DLR EnMAP 高光谱 L2A 地表反射率"
    );
    for (const awsId of [
      "science.aws.sentinel-1-grd.v1",
      "science.noaa.aws-weather.v1",
      "science.overture.maps.v1",
    ]) {
      const awsItem = items.find((i) => i.resourceId === awsId);
      assert.ok(awsItem, `expected AWS Open Data entry ${awsId}`);
      assert.strictEqual(awsItem.status, "available");
      assert.strictEqual(awsItem.accountRequirement, "free");
    }
  });

  await suite.test("8. en & ja catalog items cover era/provider/satellites cleanly without Simplified Chinese leaks", () => {
    const cjkRe = /[\u4e00-\u9fff]/;
    const enItems = buildCatalogItems(mockDescriptors, {}, "en");
    const jaItems = buildCatalogItems(mockDescriptors, {}, "ja");
    const zhItems = buildCatalogItems(mockDescriptors, {}, "zh-CN");
    const zhMap = new Map(zhItems.map((item) => [item.resourceId, item]));

    for (const item of enItems) {
      for (const [field, val] of Object.entries(item)) {
        const list = Array.isArray(val) ? val : [val];
        for (const s of list) {
          if (typeof s === "string") {
            assert.strictEqual(
              cjkRe.test(s),
              false,
              `en catalog entry ${item.resourceId}.${field} leaked CJK text: ${s}`
            );
          }
        }
      }
    }

    for (const item of jaItems) {
      const zh = zhMap.get(item.resourceId);
      for (const field of ["name", "subtitle", "band", "era", "provider", "satellites", "timeRange", "accountText", "description"] as const) {
        const jaVal = item[field];
        const zhVal = zh?.[field];
        if (typeof jaVal === "string" && cjkRe.test(jaVal)) {
          assert.notStrictEqual(
            jaVal,
            zhVal,
            `ja catalog entry ${item.resourceId}.${field} must not fall back verbatim to zh-CN: ${jaVal}`
          );
        }
      }
    }

    const xyzJa = jaItems.find((i) => i.resourceId === "auxiliary.xyz-tiles.v1");
    assert.ok(xyzJa);
    assert.ok(xyzJa.provider?.includes("高徳"));
    assert.ok(xyzJa.provider?.includes("天地図"));
    assert.ok(xyzJa.provider?.includes("カスタム"));
    assert.strictEqual(xyzJa.provider?.includes("高德"), false);
    assert.strictEqual(xyzJa.provider?.includes("天地图"), false);
    assert.strictEqual(xyzJa.provider?.includes("自定义"), false);
    assert.strictEqual(xyzJa.satellites?.includes("高德"), false);
    assert.strictEqual(xyzJa.satellites?.includes("天地图"), false);
  });
});
