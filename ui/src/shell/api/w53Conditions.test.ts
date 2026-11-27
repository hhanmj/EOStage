import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  conditionsToSearchParams,
  validateConditions,
  quickRange,
  initialGroupState,
  toggleGroup,
  sourceSummary,
  aoiSummary,
  timeSummary,
  sarSourceOptions,
  lastSearchSummary,
  toolResultLine,
  orbitDownloadResultLine,
  demStartResultLine,
  type ManualConditionsState,
} from "./manualConditionsLogic.ts";
import {
  extractBase64FromDataUrl,
  bboxArrayToBbox,
} from "./vectorAoiImport.ts";
import type { SourceDescriptor } from "./types.ts";
import { assertLocalizedSource } from "./i18nSourceTestUtils.test.ts";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

test("W5-3a Conditions Logic - Pure Module Unit Tests", async (t) => {
  // (a) conditionsToSearchParams(state) -> BuildSearchRequestParams
  await t.test("(a) conditionsToSearchParams and validateConditions", () => {
    const validState: ManualConditionsState = {
      resourceId: "science.sentinel-1.standard.v1",
      aoiGeometry: { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] },
      aoiName: "测试区域",
      startDate: "2026-09-01",
      endDate: "2026-10-01",
      productType: "SLC",
      orbitDirection: "all",
      relativeOrbit: "12",
      frame: " 65 ",
    };

    const params = conditionsToSearchParams(validState);
    assert.equal(params.resourceId, "science.sentinel-1.standard.v1");
    assert.deepEqual(params.aoiGeometry, validState.aoiGeometry);
    assert.equal(params.aoiName, "测试区域");
    assert.equal(params.startDate, "2026-09-01");
    assert.equal(params.endDate, "2026-10-01");
    assert.equal(params.productType, "SLC");
    // orbitDirection "all" -> null
    assert.equal(params.orbitDirection, null);
    // relativeOrbit string "12" -> number 12
    assert.equal(params.relativeOrbit, 12);
    // frame string " 65 " -> number 65
    assert.equal(params.frame, 65);

    // Test pass through for ascending / descending
    assert.equal(conditionsToSearchParams({ ...validState, orbitDirection: "ascending" }).orbitDirection, "ascending");
    assert.equal(conditionsToSearchParams({ ...validState, orbitDirection: "descending" }).orbitDirection, "descending");

    // Empty / whitespace relativeOrbit and frame -> null
    const emptyOrbit = conditionsToSearchParams({ ...validState, relativeOrbit: "", frame: "   " });
    assert.equal(emptyOrbit.relativeOrbit, null);
    assert.equal(emptyOrbit.frame, null);

    // No field outside BuildSearchRequestParams keys
    const allowedKeys = new Set([
      "resourceId",
      "aoiGeometry",
      "aoiName",
      "startDate",
      "endDate",
      "platforms",
      "productType",
      "polarizations",
      "orbitDirection",
      "relativeOrbit",
      "path",
      "frame",
    ]);
    for (const key of Object.keys(params)) {
      assert.ok(allowedKeys.has(key), `Key '${key}' must not be outside BuildSearchRequestParams`);
    }

    // Validation of non-integer strings
    const invalidState: ManualConditionsState = {
      relativeOrbit: "12a",
      frame: "1.5",
    };
    const errors = validateConditions(invalidState);
    assert.equal(errors.length, 2);
    assert.deepEqual(errors[0], { field: "relativeOrbit", message: "Path 需为整数" });
    assert.deepEqual(errors[1], { field: "frame", message: "Frame 需为整数" });

    // Valid state produces no errors
    assert.deepEqual(validateConditions(validState), []);
  });

  // (b) quickRange(kind, now)
  await t.test("(b) quickRange calculates local YYYY-MM-DD range with month end clamping", () => {
    // 3m with month end clamp: 2026-05-31 minus 3 months = 2026-02-28
    const date3m = new Date(2026, 4, 31);
    const range3m = quickRange("3m", date3m);
    assert.equal(range3m.endDate, "2026-05-31");
    assert.equal(range3m.startDate, "2026-02-28");

    // 6m: 2026-10-22 minus 6 months = 2026-04-22
    const date6m = new Date(2026, 9, 22);
    const range6m = quickRange("6m", date6m);
    assert.equal(range6m.endDate, "2026-10-22");
    assert.equal(range6m.startDate, "2026-04-22");

    // 1y: 2026-02-28 minus 1 year = 2025-02-28
    const date1y = new Date(2026, 1, 28);
    const range1y = quickRange("1y", date1y);
    assert.equal(range1y.endDate, "2026-02-28");
    assert.equal(range1y.startDate, "2025-02-28");

    // Leap year clamp: 2024-02-29 minus 1 year = 2023-02-28
    const dateLeap = new Date(2024, 1, 29);
    const rangeLeap = quickRange("1y", dateLeap);
    assert.equal(rangeLeap.endDate, "2024-02-29");
    assert.equal(rangeLeap.startDate, "2023-02-28");

    // 3y & 5y presets (Item 8) with normal date & leap-year clamping
    const dateNov = new Date(2026, 10, 19);
    assert.deepEqual(quickRange("3y", dateNov), {
      startDate: "2023-11-19",
      endDate: "2026-11-19",
    });
    assert.deepEqual(quickRange("5y", dateNov), {
      startDate: "2021-11-19",
      endDate: "2026-11-19",
    });
    assert.deepEqual(quickRange("3y", dateLeap), {
      startDate: "2021-02-28",
      endDate: "2024-02-29",
    });
    assert.deepEqual(quickRange("5y", dateLeap), {
      startDate: "2019-02-28",
      endDate: "2024-02-29",
    });
  });

  // (c) Group collapse state
  await t.test("(c) Group collapse state management", () => {
    const initial = initialGroupState();
    assert.deepEqual(initial, {
      source: true,
      aoi: true,
      time: true,
      orbit: true,
    });

    // Toggle flips only that group
    const toggled = toggleGroup(initial, "aoi");
    assert.equal(toggled.aoi, false);
    assert.equal(toggled.source, true);
    assert.equal(toggled.time, true);
    assert.equal(toggled.orbit, true);

    // Toggle back
    const toggledBack = toggleGroup(toggled, "aoi");
    assert.equal(toggledBack.aoi, true);

    // Plain JSON-serializable
    assert.deepEqual(JSON.parse(JSON.stringify(toggled)), toggled);
  });

  // (d) Group summaries
  await t.test("(d) Group summaries for source, AOI, and time", () => {
    // sourceSummary
    const desc: SourceDescriptor = {
      resource_id: "science.sentinel-1.standard.v1",
      group_id: "asf",
      display_name_zh: "Alaska Satellite Facility (ASF)",
      family: "sar",
      search_mode: "spatial_temporal",
      filter_schema_id: null,
      filter_schema_revision: null,
      result_cap: 1000,
      accounts: [],
      network_default: "direct",
    };
    assert.equal(sourceSummary(desc), "Alaska Satellite Facility (ASF)");
    assert.equal(sourceSummary(null), "");
    assert.equal(sourceSummary(undefined), "");

    // aoiSummary: bbox null -> { name: "未设定", sizeText: "" }
    assert.deepEqual(aoiSummary(null, "任意名称"), { name: "未设定", sizeText: "" });
    assert.deepEqual(aoiSummary(undefined), { name: "未设定", sizeText: "" });

    // aoiSummary with bbox: W at midLat using cos(lat), 1 deg lat = 111.32 km
    // e.g. south = 31.8, north = 32.0 (delta = 0.2 deg, H = 0.2 * 111.32 = 22.264 -> 22 km)
    // west = 103.5, east = 103.75 (delta = 0.25 deg, midLat = 31.9 deg, cos(31.9) = 0.84897, W = 0.25 * 111.32 * 0.84897 = 23.627 -> 24 km)
    const bbox = { west: 103.5, south: 31.8, east: 103.75, north: 32.0 };
    const aoi = aoiSummary(bbox, "茂县叠溪");
    assert.equal(aoi.name, "茂县叠溪");
    assert.equal(aoi.sizeText, "约 24 × 22 km");
    // Must never claim a shape
    assert.ok(!aoi.sizeText.includes("矩形"));

    // Tiny bbox (< 1 km)
    const tinyBbox = { west: 100.0, south: 30.0, east: 100.001, north: 30.001 };
    const tinyAoi = aoiSummary(tinyBbox, "微小选区");
    assert.equal(tinyAoi.sizeText, "约 <1 × <1 km");

    // timeSummary
    assert.equal(timeSummary("2026-09-20", "2026-10-22"), "33 天");
    assert.equal(timeSummary("2026-10-22", "2026-10-22"), "1 天");
    // Invalid or backwards dates -> ""
    assert.equal(timeSummary("2026-10-22", "2026-09-20"), "");
    assert.equal(timeSummary("invalid", "2026-10-22"), "");
    assert.equal(timeSummary(null, "2026-10-22"), "");
  });

  // (e) sarSourceOptions
  await t.test("(e) sarSourceOptions filters only family === 'sar' in input order", () => {
    const descriptors: SourceDescriptor[] = [
      {
        resource_id: "sar.sentinel1",
        group_id: "s1",
        display_name_zh: "Sentinel-1 影像",
        family: "sar",
        search_mode: "spatial_temporal",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 100,
        accounts: [],
        network_default: "direct",
      },
      {
        resource_id: "optical.landsat8",
        group_id: "l8",
        display_name_zh: "Landsat-8",
        family: "optical",
        search_mode: "spatial_temporal",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 100,
        accounts: [],
        network_default: "direct",
      },
      {
        resource_id: "sar.nisar",
        group_id: "nisar",
        display_name_zh: "NISAR",
        family: "sar",
        search_mode: "spatial_temporal",
        filter_schema_id: null,
        filter_schema_revision: null,
        result_cap: 100,
        accounts: [],
        network_default: "direct",
      },
    ];

    const options = sarSourceOptions(descriptors);
    assert.deepEqual(options, [
      { value: "sar.sentinel1", label: "Sentinel-1 影像" },
      { value: "sar.nisar", label: "NISAR" },
    ]);

    // Empty input -> empty array, no invented entries
    assert.deepEqual(sarSourceOptions([]), []);
  });

  // (f) lastSearchSummary
  await t.test("(f) lastSearchSummary formats local time and item count", () => {
    const fixedNow = new Date(2026, 9, 25, 14, 0, 0);

    // Today at 10:12
    const todaySearch = new Date(2026, 9, 25, 10, 12, 0).toISOString();
    assert.equal(lastSearchSummary(todaySearch, 8, fixedNow), "上次检索 10:12，共 8 景");

    // Yesterday or another date (October 22 at 09:30)
    const pastSearch = new Date(2026, 9, 22, 9, 30, 0).toISOString();
    assert.equal(lastSearchSummary(pastSearch, 15, fixedNow), "上次检索 10月22日 09:30，共 15 景");

    // Invalid / null searchedAt -> ""
    assert.equal(lastSearchSummary(null, 5, fixedNow), "");
    assert.equal(lastSearchSummary("", 5, fixedNow), "");
    assert.equal(lastSearchSummary("invalid-date", 5, fixedNow), "");

    // R2a: When itemCount is not a finite number, return "上次检索 HH:mm" without count
    assert.equal(lastSearchSummary(todaySearch, null, fixedNow), "上次检索 10:12");
    assert.equal(lastSearchSummary(todaySearch, undefined, fixedNow), "上次检索 10:12");
    assert.equal(lastSearchSummary(pastSearch, null, fixedNow), "上次检索 10月22日 09:30");
  });

  // (g) vectorAoiImport pure helpers (R3)
  await t.test("(g) vectorAoiImport pure helpers (R3)", () => {
    // extractBase64FromDataUrl
    const withComma = "data:application/json;base64,eyJmb28iOiJiYXIifQ==";
    assert.equal(extractBase64FromDataUrl(withComma), "eyJmb28iOiJiYXIifQ==");
    const withoutComma = "eyJmb28iOiJiYXIifQ==";
    assert.equal(extractBase64FromDataUrl(withoutComma), "eyJmb28iOiJiYXIifQ==");
    assert.equal(extractBase64FromDataUrl(""), "");

    // bboxArrayToBbox
    const validArr = [103.5, 31.8, 103.75, 32.0];
    assert.deepEqual(bboxArrayToBbox(validArr), {
      west: 103.5,
      south: 31.8,
      east: 103.75,
      north: 32.0,
      crs: "EPSG:4326",
    });

    // Invalid length
    assert.equal(bboxArrayToBbox([103.5, 31.8, 103.75]), null);
    assert.equal(bboxArrayToBbox([103.5, 31.8, 103.75, 32.0, 1.0]), null);

    // Invalid types or NaN
    assert.equal(bboxArrayToBbox([103.5, NaN, 103.75, 32.0]), null);
    assert.equal(bboxArrayToBbox([103.5, "31.8", 103.75, 32.0]), null);
    assert.equal(bboxArrayToBbox("not-an-array"), null);
    assert.equal(bboxArrayToBbox(null), null);
  });

  // (h) toolResultLine formats dem/orbit/convert results honestly (A3c)
  await t.test("(h) toolResultLine formats dem/orbit/convert results honestly (A3c)", () => {
    // 1. DEM results
    assert.equal(toolResultLine("dem", { ok: true, tile_count: 8 }), "DEM 瓦片规划完成，共 8 块");
    assert.equal(toolResultLine("dem", { ok: true }), "DEM 瓦片规划完成");
    assert.equal(toolResultLine("dem", { ok: true, tile_count: "not-a-number" }), "DEM 瓦片规划完成");
    assert.equal(toolResultLine("dem", { ok: false, code: "model_timeout" }), "模型服务响应超时");
    assert.equal(
      toolResultLine("dem", {
        ok: false,
        code: "V3_APPLICATION_INVALID_REQUEST",
        error: "standalone DEM request is invalid",
      }),
      "当前请求参数无效，请检查本次操作所需的输入后重试。"
    );
    assert.equal(
      toolResultLine("dem", { ok: false, error: "磁盘空间不足" }),
      "DEM 规划失败：磁盘空间不足"
    );
    assert.equal(toolResultLine("dem", { ok: false }), "DEM 规划失败");

    // 2. Orbit results
    assert.equal(toolResultLine("orbit", { ok: true, matched_count: 5 }), "已匹配 5 个轨道文件");
    assert.equal(toolResultLine("orbit", { ok: true }), "轨道匹配完成");
    assert.equal(toolResultLine("orbit", { ok: true, matched_count: null }), "轨道匹配完成");
    assert.equal(
      toolResultLine("orbit", { ok: false, error: { code: "model_timeout" } }),
      "模型服务响应超时"
    );
    assert.equal(toolResultLine("orbit", { ok: false }), "轨道匹配失败");

    // 3. Convert results
    assert.equal(
      toolResultLine("convert", { ok: true, output_path: "C:\\InSAR\\dem_out" }),
      "高程转换完成：C:\\InSAR\\dem_out"
    );
    assert.equal(
      toolResultLine("convert", {
        ok: true,
        plan: { sarscape_ready_dem_path: "C:\\InSAR\\dem_sarscape" },
      }),
      "高程转换完成：C:\\InSAR\\dem_sarscape"
    );
    assert.equal(toolResultLine("convert", { ok: true }), "高程转换完成");
    assert.equal(
      toolResultLine("convert", { ok: false, code: "AGENT_CHANNEL_KEY_MISSING" }),
      "未配置访问密钥"
    );
    assert.equal(toolResultLine("convert", { ok: false }), "高程转换失败");
  });

  // (i) orbitDownloadResultLine & demStartResultLine pure helpers (RED first)
  await t.test("(i) orbitDownloadResultLine & demStartResultLine pure helpers", () => {
    // 1. orbitDownloadResultLine
    assert.equal(orbitDownloadResultLine({ ok: true }, 5), "已提交 5 个场景的轨道下载");
    assert.equal(
      orbitDownloadResultLine({ ok: false, code: "model_timeout" }, 5),
      "模型服务响应超时"
    );
    assert.equal(orbitDownloadResultLine(null, 5), "轨道下载失败");
    assert.equal(orbitDownloadResultLine({ ok: false }, 5), "轨道下载失败");

    // 2. demStartResultLine
    assert.equal(
      demStartResultLine({ ok: true }, "DEM 瓦片规划完成，共 4 块"),
      "DEM 瓦片规划完成，共 4 块"
    );
    assert.equal(
      demStartResultLine({ ok: false, code: "model_timeout" }, "DEM 瓦片规划完成"),
      "模型服务响应超时"
    );
    assert.equal(demStartResultLine(null), "DEM 下载未启动");
    assert.equal(demStartResultLine({ ok: false }), "DEM 下载未启动");
  });

  // Source-level checks for ManualConditionsPanel and DesktopWorkbench
  await t.test("Source-level assertions for ManualConditionsPanel and DesktopWorkbench", () => {
    const desktopDir = path.resolve(__dirname, "../../components/desktop");

    // 1. ManualConditionsPanel.tsx checks
    const panelPath = path.join(desktopDir, "ManualConditionsPanel.tsx");
    const panelContent = fs.readFileSync(panelPath, "utf-8");

    // F1: Polarization 与卫星平台控件已按注册表能力接通
    assert.ok(
      panelContent.includes("Polarization"),
      "ManualConditionsPanel must render Polarization control when supported"
    );
    assert.ok(
      panelContent.includes("卫星"),
      "ManualConditionsPanel must render 卫星 controls when supported"
    );

    // No "即将" or disabled placeholder controls
    assert.strictEqual(
      panelContent.includes("即将"),
      false,
      "ManualConditionsPanel must have no 即将 or placeholder controls"
    );

    // 检索 button calls trigger prop (onTriggerSearch)
    assert.match(
      panelContent,
      /onClick=\{\s*onTriggerSearch\s*\}/,
      "ManualConditionsPanel must bind 检索 button to onTriggerSearch"
    );

    // 导入矢量 button and accepted extensions
    assertLocalizedSource(panelContent, "mapWorkbench.conditions.importVector", "导入矢量");
    assert.match(
      panelContent,
      /AGENT_VECTOR_EXTENSIONS/,
      "ManualConditionsPanel must reuse AGENT_VECTOR_EXTENSIONS"
    );

    // ManualConditionsPanel renders searchError prop
    assert.match(
      panelContent,
      /searchError\?: string \| null/,
      "ManualConditionsPanel must declare searchError prop"
    );
    assert.match(
      panelContent,
      /\{scientificMode === "sar" && searchError &&/,
      "ManualConditionsPanel must render searchError for SAR mode"
    );

    // Group headers are accessible button elements
    assert.match(
      panelContent,
      /<button[^>]*aria-expanded=[^>]*aria-controls=/,
      "ManualConditionsPanel must render accessible button headers with aria-expanded"
    );

    // A3a: 检索类型仅保留四项真实功能，不含光学或 GACOS 入口
    assert.strictEqual(
      panelContent.includes('value="optical"'),
      false,
      "ManualConditionsPanel must not offer optical mode"
    );
    assert.strictEqual(
      panelContent.includes('value="gacos"'),
      false,
      "ManualConditionsPanel must not offer gacos mode"
    );

    // A3b: DesktopParamDrawer.tsx 已彻底删除
    const drawerPath = path.join(desktopDir, "DesktopParamDrawer.tsx");
    assert.strictEqual(
      fs.existsSync(drawerPath),
      false,
      "DesktopParamDrawer.tsx must be deleted"
    );

    // 2. DesktopWorkbench.tsx checks
    const wbPath = path.join(desktopDir, "DesktopWorkbench.tsx");
    const wbContent = fs.readFileSync(wbPath, "utf-8");

    // DesktopWorkbench 不再引用 DesktopParamDrawer 或 planGacosRequest
    assert.strictEqual(
      wbContent.includes("DesktopParamDrawer"),
      false,
      "DesktopWorkbench must not import DesktopParamDrawer"
    );
    assert.strictEqual(
      wbContent.includes("planGacosRequest"),
      false,
      "DesktopWorkbench must not import or call planGacosRequest"
    );

    // DesktopWorkbench's workbench route renders ManualWorkbench
    assert.ok(
      wbContent.includes("<ManualWorkbench"),
      "DesktopWorkbench must render ManualWorkbench for workbench route"
    );

    // DesktopWorkbench binds onImportVectorSuccess to setWorkingGeometry
    assert.match(
      wbContent,
      /onImportVectorSuccess=\{[^}]*setWorkingGeometry/,
      "DesktopWorkbench must wire onImportVectorSuccess to setWorkingGeometry"
    );

    // DesktopWorkbench no longer renders WorkbenchMap as a full-bleed <main> floor
    assert.doesNotMatch(
      wbContent,
      /<main[^>]*>\s*<WorkbenchMap/s,
      "DesktopWorkbench must not render WorkbenchMap as a full-bleed <main> floor"
    );
  });

  await t.test("(i) resolveEffectiveNumericBounds + validateConditions reject out-of-range SAR Path/RelativeOrbit", async () => {
    const { sarFilterProfile, resolveEffectiveNumericBounds } = await import("./sarFilterProfile.ts");
    const s1Profile = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.ok(s1Profile?.relativeOrbit, "Sentinel-1 must expose relativeOrbit profile bounds");
    const s1OrbitBounds = resolveEffectiveNumericBounds(s1Profile!.relativeOrbit, []);
    assert.equal(s1OrbitBounds.min, 1);
    assert.equal(s1OrbitBounds.max, 175);

    const invalidS1 = validateConditions({
      relativeOrbit: "999",
      relativeOrbitProfile: s1OrbitBounds,
    });
    assert.equal(invalidS1.length, 1);
    assert.equal(invalidS1[0].field, "relativeOrbit");
    assert.match(invalidS1[0].message, /1.*175/);

    const validS1 = validateConditions({
      relativeOrbit: "100",
      relativeOrbitProfile: s1OrbitBounds,
    });
    assert.equal(validS1.length, 0);

    const alosProfile = sarFilterProfile("science.alos.palsar.v1");
    assert.ok(alosProfile?.path && alosProfile?.frame, "ALOS PALSAR must expose path & frame profile bounds");
    const invalidAlos = validateConditions({
      path: "999",
      pathProfile: resolveEffectiveNumericBounds(alosProfile!.path, []),
      frame: "9999",
      frameProfile: {
        ...alosProfile!.frame!,
        ...resolveEffectiveNumericBounds(alosProfile!.frame!, []),
      },
    });
    assert.ok(invalidAlos.some((e) => e.field === "path"));
    assert.ok(invalidAlos.some((e) => e.field === "frame"));
  });
});
