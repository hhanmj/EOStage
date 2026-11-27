import test from "node:test";
import assert from "node:assert/strict";
import type { SourceDescriptor } from "./types.ts";
import type { SceneRow } from "../../lib/bridge.ts";
import {
  sourceOptions,
  conditionsKind,
  opticalFilters,
  isSourceSearchOnly,
  utcDayRange,
} from "./sourceConditions.ts";
import { buildUnifiedSearchRequest } from "./searchRequest.ts";
import { buildResultRowBadges, extractCloudCover } from "./rowProjection.ts";
import { sortKeysFor, sortScenes } from "./resultListLogic.ts";
import { groupOrbitTitle } from "./manualConditionsLogic.ts";
import { sarFilterProfile } from "./sarFilterProfile.ts";

test("W5-3a-F2: Optical Sources, Schema Conditions & Result Row Suite", async (t) => {
  const sampleDescriptors: SourceDescriptor[] = [
    {
      resource_id: "science.sentinel-1.standard.v1",
      group_id: "sentinel-1",
      display_name_zh: "Sentinel-1 常用",
      family: "sar",
      search_mode: "sar",
      filter_schema_id: null,
      filter_schema_revision: null,
      result_cap: 250,
      accounts: ["earthdata"],
      network_default: "direct",
      availability: "available",
      availability_reason_zh: null,
    },
    {
      resource_id: "science.alos.palsar.v1",
      group_id: "alos",
      display_name_zh: "ALOS PALSAR",
      family: "sar",
      search_mode: "sar",
      filter_schema_id: null,
      filter_schema_revision: null,
      result_cap: 250,
      accounts: ["earthdata"],
      network_default: "direct",
      availability: "unavailable",
      availability_reason_zh: "服务维护中",
    },
    {
      resource_id: "science.sentinel-2.l2a.v1",
      group_id: "sentinel-2",
      display_name_zh: "Sentinel-2 多光谱",
      family: "optical",
      search_mode: "catalogue_intent",
      filter_schema_id: "search.optical.sentinel-2.l2a.v1",
      filter_schema_revision: 1,
      result_cap: 100,
      accounts: ["cdse"],
      network_default: "direct",
      availability: "available",
      availability_reason_zh: null,
    },
    {
      resource_id: "science.fake.optical.v1",
      group_id: "fake",
      display_name_zh: "未知光学",
      family: "optical",
      search_mode: "catalogue_intent",
      filter_schema_id: "non.existent.schema.id",
      filter_schema_revision: 1,
      result_cap: 100,
      accounts: ["cdse"],
      network_default: "direct",
      availability: "available",
      availability_reason_zh: null,
    },
    {
      resource_id: "dem.cop30.v1",
      group_id: "dem",
      display_name_zh: "Copernicus DEM 30m",
      family: "dem",
      search_mode: "auxiliary",
      filter_schema_id: null,
      filter_schema_revision: null,
      result_cap: 1,
      accounts: ["earthdata"],
      network_default: "direct",
      availability: "available",
      availability_reason_zh: null,
    },
  ];

  // 1. sourceOptions 分组排序与过滤
  await t.test("1. sourceOptions groups SAR then optical and respects schema catalog", () => {
    const groups = sourceOptions(sampleDescriptors);
    assert.equal(groups.length, 2, "应输出 SAR 和 光学 两个分组");
    assert.equal(groups[0].group, "SAR");
    assert.equal(groups[1].group, "光学");

    // SAR 分组：包含 2 个源，unavailable 项设置 disabled 与 title
    const sarOpts = groups[0].options;
    assert.equal(sarOpts.length, 2);
    assert.equal(sarOpts[0].value, "science.sentinel-1.standard.v1");
    assert.equal(sarOpts[0].disabled, false);
    assert.equal(sarOpts[1].value, "science.alos.palsar.v1");
    assert.equal(sarOpts[1].disabled, true);
    assert.equal(sarOpts[1].title, "服务维护中");

    // 光学分组：仅包含在 SEARCH_FILTER_SCHEMA_CATALOG 中存在的 schema，剔除非法 schema
    const opticalOpts = groups[1].options;
    assert.equal(opticalOpts.length, 1);
    assert.equal(opticalOpts[0].value, "science.sentinel-2.l2a.v1");
    assert.equal(opticalOpts[0].disabled, false);

    // 空列表安全返回
    assert.deepEqual(sourceOptions([]), []);

    // ASF 已下线的 ERS / RADARSAT-1 / JERS-1 不在界面列出（Owner 2026-10-31）
    const removed = ["science.ers-1-2.sar.v1", "science.radarsat-1.sar.v1", "science.jers-1.sar.v1"].map(
      (resource_id) => ({ ...sampleDescriptors[0], resource_id, availability: "unavailable" as const })
    );
    const withRemoved = sourceOptions([...sampleDescriptors, ...removed]);
    const listed = withRemoved.flatMap((g) => g.options.map((o) => o.value));
    for (const r of removed) assert.ok(!listed.includes(r.resource_id), `${r.resource_id} 不应出现在下拉框`);
    assert.equal(withRemoved[0].options.length, 2);
  });

  // 2. conditionsKind 类型判别
  await t.test("2. conditionsKind accurately identifies sar vs optical vs null", () => {
    assert.equal(conditionsKind("science.sentinel-1.standard.v1", sampleDescriptors), "sar");
    assert.equal(conditionsKind("science.sentinel-2.l2a.v1", sampleDescriptors), "optical");
    assert.equal(conditionsKind("science.fake.optical.v1", sampleDescriptors), null);
    assert.equal(conditionsKind("dem.cop30.v1", sampleDescriptors), null);
    assert.equal(conditionsKind("unknown.id", sampleDescriptors), null);
  });

  // 3. opticalFilters 校验与构建
  await t.test("3. opticalFilters builds fixed predicates, multi_enum, cloud_cover, without datetime in filters", () => {
    const schemaId = "search.optical.sentinel-2.l2a.v1";

    // (a) 合法输入：platforms + cloud_cover + 有效时间范围
    const res = opticalFilters(
      schemaId,
      {
        platform: ["sentinel_2a"],
        cloud_cover: "20",
      },
      "2026-07-01",
      "2026-07-10"
    );

    assert.equal(res.ok, true);
    if (!res.ok) return;

    // filters 恰好包含 fixed predicates + spacecraft + cloud_cover_lte
    assert.deepEqual(res.filters, {
      product_type: "S2MSI2A",
      spacecraft: ["sentinel_2a"],
      cloud_cover_lte: 20,
    });
    // filters 严禁包含 datetime 或 dataset
    assert.equal("datetime" in res.filters, false);
    assert.equal("dataset" in res.filters, false);

    // timeRange 包含标准化 UTC 时间戳 (含当天闭区间 23:59:59Z)
    assert.deepEqual(res.timeRange, {
      start: "2026-07-01T00:00:00Z",
      end: "2026-07-10T23:59:59Z",
    });

    // (b) 时间均为空 -> timeRange 为 null
    const resNoTime = opticalFilters(
      schemaId,
      { platform: ["sentinel_2b"] },
      "",
      ""
    );
    assert.equal(resNoTime.ok, true);
    if (resNoTime.ok) {
      assert.equal(resNoTime.timeRange, null);
    }

    // (c) 仅有一个日期 -> 报错拒绝
    const resOneDate = opticalFilters(schemaId, {}, "2026-07-01", "");
    assert.equal(resOneDate.ok, false);
    if (!resOneDate.ok) {
      assert.match(resOneDate.error, /开始和结束日期/);
    }

    // (d) 开始晚于结束 -> 报错拒绝
    const resInverted = opticalFilters(schemaId, {}, "2026-07-10", "2026-07-01");
    assert.equal(resInverted.ok, false);
    if (!resInverted.ok) {
      assert.match(resInverted.error, /结束日期不能早于开始日期/);
    }

    // (e) 未知 schemaId -> 报错拒绝
    const resUnknown = opticalFilters("non.existent.schema", {}, "", "");
    assert.equal(resUnknown.ok, false);
    if (!resUnknown.ok) {
      assert.match(resUnknown.error, /未知的筛选 schema/);
    }
  });

  // 4. buildUnifiedSearchRequest 对光学源与 SAR 源的分流支持
  await t.test("4. buildUnifiedSearchRequest routes optical sources cleanly without SAR keys", () => {
    const aoi = {
      type: "Polygon",
      coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
    };

    // (a) 光学源请求
    const opticalReq = buildUnifiedSearchRequest({
      resourceId: "science.sentinel-2.l2a.v1",
      aoiGeometry: aoi,
      aoiName: "光学测试区",
      startDate: "2026-07-01",
      endDate: "2026-07-10",
      opticalDraft: {
        platform: ["sentinel_2a", "sentinel_2b"],
        cloud_cover: "15.5",
      },
    });

    assert.equal(opticalReq.resource_id, "science.sentinel-2.l2a.v1");
    assert.equal(opticalReq.aoi_name, "光学测试区");
    assert.deepEqual(opticalReq.filters, {
      product_type: "S2MSI2A",
      spacecraft: ["sentinel_2a", "sentinel_2b"],
      cloud_cover_lte: 15.5,
    });
    // 绝无 SAR 键
    assert.equal("dataset" in opticalReq.filters, false);
    assert.equal("processing_levels" in opticalReq.filters, false);
    assert.equal("relative_orbit" in opticalReq.filters, false);
    assert.equal("flight_direction" in opticalReq.filters, false);

    // time_range 为标准化 UTC (含当天闭区间 23:59:59Z)
    assert.deepEqual(opticalReq.time_range, {
      start: "2026-07-01T00:00:00Z",
      end: "2026-07-10T23:59:59Z",
    });

    // (b) SAR 源请求保持不变
    const sarReq = buildUnifiedSearchRequest({
      resourceId: "science.sentinel-1.standard.v1",
      aoiGeometry: aoi,
      aoiName: "SAR测试区",
      startDate: "2026-07-01",
      endDate: "2026-07-10",
      productType: "SLC",
    });
    assert.equal(sarReq.resource_id, "science.sentinel-1.standard.v1");
    assert.equal(sarReq.filters.dataset, "SENTINEL-1");
    assert.deepEqual(sarReq.filters.processing_levels, ["SLC"]);
  });

  // 5. extractCloudCover & buildResultRowBadges 分族呈现
  await t.test("5. optical row badges include date, platform, cloud cover, product type, size; never Unknown orbit/frame", () => {
    const opticalRow: SceneRow = {
      scene_id: "S2A_MSIL2A_20260701_test",
      product_key: "S2A_MSIL2A_20260701_test",
      product_id: "S2A_MSIL2A_20260701_test",
      provider: "copernicus.dataspace",
      file_name: "S2A_MSIL2A_20260701_test.SAFE",
      acquisition_datetime: "2026-07-01T10:30:00Z",
      footprint_geojson: null,
      platform: "Sentinel-2A",
      processing_level: "L2A",
      product_type: "S2MSI2A",
      beam_mode: "",
      polarization: "",
      orbit_direction: "",
      relative_orbit: null,
      frame: null,
      absolute_orbit: null,
      path: null,
      file_size_remote: 1024 * 1024 * 500, // 500 MB
      has_url: false,
      native_metadata: {
        properties: {
          cloud_cover: 4.2,
          product_type: "S2MSI2A",
        },
      },
    };

    assert.equal(extractCloudCover(opticalRow), 4.2);

    const badges = buildResultRowBadges(opticalRow, "optical");
    const keys = badges.map((b) => b.key);
    assert.deepEqual(keys, ["date", "platform", "cloud", "level", "size"]);

    const cloudBadge = badges.find((b) => b.key === "cloud");
    assert.ok(cloudBadge);
    assert.equal(cloudBadge.value, "云量 4.2%");

    // 严禁包含 orbit、frame、direction 等 SAR 徽章
    assert.equal(badges.some((b) => b.value.includes("Path")), false);
    assert.equal(badges.some((b) => b.value.includes("Frame")), false);
    assert.equal(badges.some((b) => b.value === "Unknown"), false);

    // 无真实云量时省略云量徽章
    const noCloudRow: SceneRow = {
      ...opticalRow,
      native_metadata: { properties: {} },
    };
    assert.equal(extractCloudCover(noCloudRow), null);
    const noCloudBadges = buildResultRowBadges(noCloudRow, "optical");
    assert.equal(noCloudBadges.some((b) => b.key === "cloud"), false);
  });

  // 6. sortKeysFor & sortScenes "cloud" 排序
  await t.test("6. sortKeysFor adapts by kind and sortScenes handles cloud key", () => {
    // optical sort keys
    const optKeysWithCloud = sortKeysFor("optical", true);
    assert.deepEqual(
      optKeysWithCloud.map((k) => k.key),
      ["date", "cloud", "size"]
    );

    const optKeysNoCloud = sortKeysFor("optical", false);
    assert.deepEqual(
      optKeysNoCloud.map((k) => k.key),
      ["date", "size"]
    );

    // sar sort keys
    const sarKeys = sortKeysFor("sar");
    assert.deepEqual(
      sarKeys.map((k) => k.key),
      ["date", "relativeOrbit", "frame", "size"]
    );

    // sortScenes by cloud
    const r1: SceneRow = {
      scene_id: "s1",
      product_key: "s1",
      product_id: "s1",
      provider: "p",
      file_name: "f1",
      acquisition_datetime: "2026-07-01",
      footprint_geojson: null,
      platform: "S2A",
      processing_level: null,
      product_type: "",
      beam_mode: "",
      polarization: "",
      orbit_direction: "",
      relative_orbit: null,
      frame: null,
      absolute_orbit: null,
      path: null,
      file_size_remote: null,
      has_url: false,
      native_metadata: { properties: { cloud_cover: 25.0 } },
    };
    const r2: SceneRow = {
      ...r1,
      scene_id: "s2",
      native_metadata: { properties: { cloud_cover: 5.0 } },
    };
    const r3: SceneRow = {
      ...r1,
      scene_id: "s3",
      native_metadata: { properties: {} }, // 缺失
    };

    const asc = sortScenes([r1, r2, r3], "cloud", "asc");
    assert.deepEqual(
      asc.map((r) => r.scene_id),
      ["s2", "s1", "s3"],
      "云量升序：小在前，缺失垫底"
    );

    const desc = sortScenes([r1, r2, r3], "cloud", "desc");
    assert.deepEqual(
      desc.map((r) => r.scene_id),
      ["r1" in { r1 } ? "s1" : "", "s2", "s3"],
      "云量降序：大在前，缺失垫底"
    );
  });

  // 7. isSourceSearchOnly 下载能力纯判断
  await t.test("7. isSourceSearchOnly detects delivery capability accurately", () => {
    // science.sentinel-2.l2a.v1 声明了 download / asset_set 等交付能力
    assert.equal(isSourceSearchOnly("science.sentinel-2.l2a.v1"), false);
    // science.sentinel-1.standard.v1 是 SAR 源（不是 optical search-only）
    assert.equal(isSourceSearchOnly("science.sentinel-1.standard.v1"), false);
    // science.dlr.enmap.l2a.v1 因归档受 DLR EOC CAS SSO 保护而严格声明为 search-only
    assert.equal(isSourceSearchOnly("science.dlr.enmap.l2a.v1"), true);
    assert.equal(isSourceSearchOnly("science.nasa.emit.l2a.v1"), false);
  });

  // 8. utcDayRange 纯函数：配对要求、RFC 3339 闭区间与先后顺序
  await t.test("8. utcDayRange pure helper enforces pair requirement, RFC3339 day range, and order", () => {
    // 全空
    assert.deepEqual(utcDayRange("", ""), { ok: true, range: null });
    assert.deepEqual(utcDayRange(null, null), { ok: true, range: null });
    assert.deepEqual(utcDayRange("  ", ""), { ok: true, range: null });

    // 单边填写报错
    assert.deepEqual(utcDayRange("2026-07-01", ""), {
      ok: false,
      error: "开始和结束日期需同时填写，或都留空",
    });
    assert.deepEqual(utcDayRange(null, "2026-07-10"), {
      ok: false,
      error: "开始和结束日期需同时填写，或都留空",
    });

    // 开始晚于结束报错
    assert.deepEqual(utcDayRange("2026-07-10", "2026-07-01"), {
      ok: false,
      error: "结束日期不能早于开始日期。",
    });

    // 同一天（含当天全天闭区间）
    assert.deepEqual(utcDayRange("2026-07-01", "2026-07-01"), {
      ok: true,
      range: {
        start: "2026-07-01T00:00:00Z",
        end: "2026-07-01T23:59:59Z",
      },
    });

    // 跨天正常区间
    assert.deepEqual(utcDayRange("2026-07-01", "2026-07-10"), {
      ok: true,
      range: {
        start: "2026-07-01T00:00:00Z",
        end: "2026-07-10T23:59:59Z",
      },
    });
  });

  // 9. opticalFilters 支持同一天单日查询且包含当天
  await t.test("9. opticalFilters supports single day query with inclusive RFC3339 day bounds", () => {
    const singleDayRes = opticalFilters(
      "search.optical.sentinel-2.l2a.v1",
      {},
      "2026-07-01",
      "2026-07-01"
    );
    assert.equal(singleDayRes.ok, true);
    if (singleDayRes.ok) {
      assert.deepEqual(singleDayRes.timeRange, {
        start: "2026-07-01T00:00:00Z",
        end: "2026-07-01T23:59:59Z",
      });
    }

    // 单边报错
    const partialRes = opticalFilters(
      "search.optical.sentinel-2.l2a.v1",
      {},
      "2026-07-01",
      ""
    );
    assert.equal(partialRes.ok, false);
    if (!partialRes.ok) {
      assert.equal(partialRes.error, "开始和结束日期需同时填写，或都留空");
    }
  });

  // 10. buildUnifiedSearchRequest 对 typed SAR 发送 RFC 3339，对 legacy SAR 保持 YYYY-MM-DD
  await t.test("10. buildUnifiedSearchRequest handles typed vs legacy SAR dates correctly", () => {
    const dummyAoi = { type: "Polygon", coordinates: [[[0, 0], [1, 0], [1, 1], [0, 1], [0, 0]]] };

    // Typed SAR: NISAR L
    const typedReq = buildUnifiedSearchRequest({
      resourceId: "science.nisar.l.rslc.provisional.v1",
      aoiGeometry: dummyAoi,
      aoiName: "Test Area",
      startDate: "2026-07-01",
      endDate: "2026-07-10",
    });
    assert.deepEqual(typedReq.time_range, {
      start: "2026-07-01T00:00:00Z",
      end: "2026-07-10T23:59:59Z",
    });

    // Typed SAR: 单边输入必须抛出错误
    assert.throws(
      () => {
        buildUnifiedSearchRequest({
          resourceId: "science.nisar.l.rslc.provisional.v1",
          aoiGeometry: dummyAoi,
          aoiName: "Test Area",
          startDate: "2026-07-01",
          endDate: "",
        });
      },
      /开始和结束日期需同时填写，或都留空/
    );

    // Legacy SAR: Sentinel-1 保持 YYYY-MM-DD
    const legacyReq = buildUnifiedSearchRequest({
      resourceId: "science.sentinel-1.standard.v1",
      aoiGeometry: dummyAoi,
      aoiName: "Test Area",
      startDate: "2026-07-01",
      endDate: "2026-07-10",
    });
    assert.deepEqual(legacyReq.time_range, {
      start: "2026-07-01",
      end: "2026-07-10",
    });
  });

  // 11. groupOrbitTitle 纯函数依据实际可用控件动态生成标题
  await t.test("11. groupOrbitTitle adapts title to rendered controls", () => {
    const nisarSProfile = sarFilterProfile("science.nisar.s.ssar.rslc.v1");
    assert.equal(groupOrbitTitle(nisarSProfile), "极化");

    const s1Profile = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.equal(groupOrbitTitle(s1Profile), "轨道 / Frame / 极化");

    // 仅有轨道无极化
    const orbitOnlyProfile = {
      kind: "typed" as const,
      selector: "TEST",
      platforms: [],
      productTypes: [],
      polarization: null,
      flightDirection: true,
      relativeOrbit: { key: "relative_orbit" },
      path: null,
      frame: null,
    };
    assert.equal(groupOrbitTitle(orbitOnlyProfile), "轨道 / Frame");

    // 空兜底
    assert.equal(groupOrbitTitle(null), "轨道 / Frame");
  });

  // 12. Issue #55: 全量 13 项 DEM 数据源选项与 formalDemIdentity 完整对齐（含 Ellipsoidal 与 ASTER GDEM v3）
  await t.test("12. all 13 DEM datasets resolve options, providers, and formalDemIdentity cleanly", async () => {
    const { DEM_DATASET_OPTIONS, formalDemIdentity, resolvedDemProvider } = await import("../../lib/demSources.ts");

    const expected13: Array<{ dataset: string; provider: string; resourceId: string; capabilityId: string }> = [
      { dataset: "COP30", provider: "COPERNICUS_AWS", resourceId: "dem.cop30.v1", capabilityId: "capability.dem.cop30.aws-download.v1" },
      { dataset: "COP90", provider: "COPERNICUS_AWS", resourceId: "dem.cop90.v1", capabilityId: "capability.dem.cop90.aws-download.v1" },
      { dataset: "FABDEM", provider: "FABDEM_PUBLIC", resourceId: "dem.fabdem.v1", capabilityId: "capability.dem.fabdem.download.v1" },
      { dataset: "GEM15_DTM", provider: "GEM15_PUBLIC", resourceId: "dem.gem15-dtm.v1", capabilityId: "capability.dem.gem15-dtm.download.v1" },
      { dataset: "GEM15_DSM", provider: "GEM15_PUBLIC", resourceId: "dem.gem15-dsm.v1", capabilityId: "capability.dem.gem15-dsm.download.v1" },
      { dataset: "NASADEM", provider: "OPENTOPOGRAPHY", resourceId: "dem.nasadem.v1", capabilityId: "capability.dem.nasadem.download.v1" },
      { dataset: "SRTM_GL3", provider: "OPENTOPOGRAPHY", resourceId: "dem.srtm-gl3.v1", capabilityId: "capability.dem.srtm-gl3.download.v1" },
      { dataset: "SRTM_GL1", provider: "OPENTOPOGRAPHY", resourceId: "dem.srtm-gl1.v1", capabilityId: "capability.dem.srtm-gl1.download.v1" },
      { dataset: "AW3D30", provider: "OPENTOPOGRAPHY", resourceId: "dem.aw3d30.v1", capabilityId: "capability.dem.aw3d30.download.v1" },
      { dataset: "SRTM_GL1_ELLIPSOIDAL", provider: "OPENTOPOGRAPHY", resourceId: "dem.srtm-gl1-ellipsoidal.v1", capabilityId: "capability.dem.srtm-gl1-ellipsoidal.download.v1" },
      { dataset: "AW3D30_ELLIPSOIDAL", provider: "OPENTOPOGRAPHY", resourceId: "dem.aw3d30-ellipsoidal.v1", capabilityId: "capability.dem.aw3d30-ellipsoidal.download.v1" },
      { dataset: "NISAR_DEM", provider: "NISAR_DEM_ASF", resourceId: "dem.nisar-dem.v1", capabilityId: "capability.dem.nisar-dem.download.v1" },
      { dataset: "ASTER_GDEM_V3", provider: "EARTHDATA", resourceId: "dem.aster-gdem.v3", capabilityId: "capability.dem.aster-gdem-v3.download.v1" },
    ];

    const optionValues = new Set(DEM_DATASET_OPTIONS.map((o) => o.value));
    for (const entry of expected13) {
      assert.equal(optionValues.has(entry.dataset), true, `missing DEM_DATASET_OPTIONS entry: ${entry.dataset}`);
      assert.equal(resolvedDemProvider(entry.dataset), entry.provider, `provider mismatch for ${entry.dataset}`);
      const id = formalDemIdentity(entry.dataset);
      assert.ok(id, `formalDemIdentity returned null for ${entry.dataset}`);
      assert.equal(id.resource_id, entry.resourceId);
      assert.equal(id.capability_id, entry.capabilityId);
      assert.equal(id.provider, entry.provider);
    }
  });
});
