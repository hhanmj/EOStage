import test from "node:test";
import assert from "node:assert/strict";
import registryJson from "../../../../contracts/v3/resource-registry.v1.json" with { type: "json" };
import {
  sarFilterProfile,
  buildSarFilters,
  cleanStaleFilterValues,
  visibleFilterFields,
  allowedPolarizationsForBeamMode,
  type SarFilterInput,
} from "./sarFilterProfile.ts";
import { buildUnifiedSearchRequest } from "./searchRequest.ts";

const SAR_SEARCH_QUERY_KEYS = new Set([
  "provider",
  "dataset",
  "platforms",
  "instrument",
  "processing_levels",
  "product_families",
  "beam_modes",
  "polarizations",
  "flight_direction",
  "look_direction",
  "relative_orbit",
  "frame",
  "asf_frame",
  "path",
  "start",
  "end",
  "track_range",
  "frame_range",
  "aoi_geojson",
  "aoi_wkt",
  "bbox",
  "max_results",
  "burst",
]);

test("W5-3a-F1: Per-source SAR Filter Profile & Search Request Suite", async (t) => {
  // 1. Sentinel-1 standard: GRD 必须展开为其家族所有真实处理级别，并根据选中的极化收窄为兼容子集
  await t.test("1. Sentinel-1 GRD narrows processing levels based on chosen polarization", () => {
    const profile = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.ok(profile !== null, "Sentinel-1 standard profile 必须存在");
    assert.equal(profile.kind, "legacy");
    assert.equal(profile.selector, "SENTINEL-1");

    const grdType = profile.productTypes.find((pt) => pt.label === "GRD");
    assert.ok(grdType !== undefined, "必须解析出 GRD 产品类型");
    assert.deepEqual(grdType.levels, ["GRD_HD", "GRD_HS", "GRD_MD", "GRD_MS"]);

    // (a) 无极化时：发送全部 GRD 级别
    const filtersNoPol = buildSarFilters(profile, { productType: "GRD" });
    assert.equal(filtersNoPol.dataset, "SENTINEL-1");
    assert.deepEqual(filtersNoPol.processing_levels, [
      "GRD_HD",
      "GRD_HS",
      "GRD_MD",
      "GRD_MS",
    ]);

    // (b) 双极化（含 '+' 如 VV+VH）：收窄为双极化专属级别 [GRD_HD, GRD_MD]
    const filtersDual = buildSarFilters(profile, {
      productType: "GRD",
      polarizations: ["VV+VH"],
    });
    assert.deepEqual(filtersDual.processing_levels, ["GRD_HD", "GRD_MD"]);
    assert.deepEqual(filtersDual.polarizations, ["VV+VH"]);

    // (c) 单极化（如 VV）：收窄为单极化专属级别 [GRD_HS, GRD_MS]
    const filtersSingle = buildSarFilters(profile, {
      productType: "GRD",
      polarizations: ["VV"],
    });
    assert.deepEqual(filtersSingle.processing_levels, ["GRD_HS", "GRD_MS"]);
    assert.deepEqual(filtersSingle.polarizations, ["VV"]);

    // (d) 非 GRD 产品类型（如 SLC）不受极化影响，级别保持不变
    const filtersSlc = buildSarFilters(profile, {
      productType: "SLC",
      polarizations: ["VV"],
    });
    assert.deepEqual(filtersSlc.processing_levels, ["SLC"]);
    assert.deepEqual(filtersSlc.polarizations, ["VV"]);
  });

  // 2. Sentinel-1 platforms 过滤透传
  await t.test("2. Sentinel-1 platforms pass-through", () => {
    const profile = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.ok(profile !== null);
    assert.deepEqual(profile.platforms, [
      "Sentinel-1A",
      "Sentinel-1B",
      "Sentinel-1C",
      "Sentinel-1D",
    ]);

    const filters = buildSarFilters(profile, {
      platforms: ["Sentinel-1A", "Sentinel-1C"],
    });
    assert.deepEqual(filters.platforms, ["Sentinel-1A", "Sentinel-1C"]);

    // 未指定 platform 时不应输出 platforms 字段
    const noPlatformFilters = buildSarFilters(profile, { platforms: [] });
    assert.equal("platforms" in noPlatformFilters, false);
  });

  // 3. ALOS-2: typed 目录，极化组合仅限 ["HH", "HH+HV"]，绝不输出 dataset/processing_levels/relative_orbit
  await t.test("3. ALOS-2 typed profile outputs path, frame, and valid polarization combinations", () => {
    const profile = sarFilterProfile("science.alos-2.palsar-2.scansar-l1.1.v1");
    assert.ok(profile !== null);
    assert.equal(profile.kind, "typed");
    assert.equal(profile.selector, "ALOS-2");
    assert.equal(profile.relativeOrbit, null, "ALOS-2 不支持 relative_orbit");
    assert.ok(profile.path !== null && profile.path.key === "path");
    assert.ok(profile.frame !== null && profile.frame.key === "frame");
    assert.equal(profile.platforms.length, 0, "typed profile 不暴露平台选择");
    assert.equal(profile.productTypes.length, 0, "typed profile 不暴露 legacy 处理级别");

    // ALOS-2 极化选项严格为 ["HH", "HH+HV"]（后端 query.rs:203-218 严格要求）
    assert.ok(profile.polarization !== null);
    assert.deepEqual(profile.polarization.options, ["HH", "HH+HV"]);

    // 选中 HH 选项 -> 发射 ["HH"]
    const filtersHh = buildSarFilters(profile, { polarizations: ["HH"] });
    assert.deepEqual(filtersHh.polarizations, ["HH"]);

    // 选中 HH+HV 组合 -> 发射 ["HH", "HV"]
    const filtersCombined = buildSarFilters(profile, {
      path: 42,
      relativeOrbit: 128, // 即使传入也不得输出
      productType: "ScanSAR",
      orbitDirection: "ascending",
      polarizations: ["HH+HV"],
      frame: 105,
    });

    assert.equal("dataset" in filtersCombined, false, "ALOS-2 严禁包含 dataset 键");
    assert.equal("processing_levels" in filtersCombined, false, "ALOS-2 严禁包含 processing_levels 键");
    assert.equal("platforms" in filtersCombined, false, "ALOS-2 严禁包含 platforms 键");
    assert.equal("relative_orbit" in filtersCombined, false, "ALOS-2 严禁包含 relative_orbit 键");
    assert.equal(filtersCombined.path, 42);
    assert.equal(filtersCombined.frame, 105);
    assert.equal(filtersCombined.flight_direction, "ASCENDING");
    assert.deepEqual(filtersCombined.polarizations, ["HH", "HV"]);

    // 单独的 "HV" 绝不被发射（如果传入无效值，被安全过滤掉）
    const filtersInvalidHv = buildSarFilters(profile, { polarizations: ["HV"] });
    assert.equal("polarizations" in filtersInvalidHv, false, "单独的 HV 绝不能被发射");
  });

  // 4. NISAR L: 仅暴露 relative_orbit 与 frame
  await t.test("4. NISAR L exposes relative_orbit and frame, uses frequency_a_polarization string", () => {
    const profile = sarFilterProfile("science.nisar.l.rslc.provisional.v1");
    assert.ok(profile !== null);
    assert.equal(profile.kind, "typed");
    assert.equal(profile.selector, "NISAR");
    assert.ok(profile.relativeOrbit !== null && profile.relativeOrbit.key === "relative_orbit");
    assert.deepEqual(profile.path, null, "NISAR L 暴露 relative_orbit 而非 path");
    assert.ok(profile.frame !== null && profile.frame.key === "frame");
    assert.ok(profile.polarization !== null);
    assert.equal(profile.polarization.key, "frequency_a_polarization");
    assert.equal(profile.polarization.multiple, false);

    const filters = buildSarFilters(profile, {
      relativeOrbit: 150,
      frame: 80,
      orbitDirection: "DESCENDING",
      polarizations: ["HH+HV"],
    });

    assert.equal("dataset" in filters, false);
    assert.equal("processing_levels" in filters, false);
    assert.equal("platforms" in filters, false);
    assert.equal(filters.relative_orbit, 150);
    assert.equal(filters.frame, 80);
    assert.equal(filters.flight_direction, "DESCENDING");
    assert.equal(filters.frequency_a_polarization, "HH+HV");
  });

  // 5. 属性测试：所有 typed SAR 资源，输出键必 ⊆ supported_keys ∪ fixed
  await t.test("5. Property test: all typed SAR resources emit keys ⊆ supported_keys ∪ fixed", () => {
    const typedResources = (registryJson.resources as any[]).filter(
      (r) => r.science?.sar_search?.projection_kind === "typed_catalog_profile"
    );
    assert.ok(typedResources.length >= 3, "必须包含至少 3 个 typed SAR 数据源");

    for (const r of typedResources) {
      const rid = r.common.resource_id;
      const profile = sarFilterProfile(rid);
      assert.ok(profile !== null, `资源 ${rid} 必须能够提取 profile`);
      assert.equal(profile.kind, "typed");

      const sarSearch = r.science.sar_search;
      const supportedKeys = new Set(sarSearch.source_filters?.supported_keys ?? []);
      const fixedKeys = new Set(Object.keys(sarSearch.source_filters?.fixed ?? {}));
      const allowedKeys = new Set([...supportedKeys, ...fixedKeys]);

      const filters = buildSarFilters(profile, {
        platforms: ["SENTINEL-1A"],
        productType: "SLC",
        orbitDirection: "ASCENDING",
        relativeOrbit: 100,
        path: 200,
        frame: 300,
        polarizations: profile.polarization?.options.slice(0, 1) ?? ["HH"],
      });

      // 绝不能含有 dataset、processing_levels、platforms
      assert.equal("dataset" in filters, false, `${rid} typed 检索严禁含有 dataset`);
      assert.equal("processing_levels" in filters, false, `${rid} typed 检索严禁含有 processing_levels`);
      assert.equal("platforms" in filters, false, `${rid} typed 检索严禁含有 platforms`);

      for (const key of Object.keys(filters)) {
        assert.ok(
          allowedKeys.has(key),
          `资源 ${rid} 的 filters 键 '${key}' 必须在 supported_keys ∪ fixed 内`
        );
      }
    }
  });

  // 6. 属性测试：所有 legacy SAR 资源，输出键必 ⊆ SarSearchQuery 字段
  await t.test("6. Property test: all legacy SAR resources emit keys ⊆ SarSearchQuery fields", () => {
    const legacyResources = (registryJson.resources as any[]).filter(
      (r) => r.science?.sar_search?.projection_kind === "legacy_asf_search"
    );
    assert.ok(legacyResources.length >= 7, "必须包含至少 7 个 legacy SAR 数据源");

    for (const r of legacyResources) {
      const rid = r.common.resource_id;
      const profile = sarFilterProfile(rid);
      assert.ok(profile !== null, `资源 ${rid} 必须能够提取 profile`);
      assert.equal(profile.kind, "legacy");

      const filters = buildSarFilters(profile, {
        platforms: profile.platforms.slice(0, 1),
        productType: profile.productTypes[0]?.label,
        orbitDirection: "ASCENDING",
        relativeOrbit: 100,
        path: 200,
        frame: 300,
        polarizations: profile.polarization?.options.slice(0, 1),
      });

      assert.equal(filters.dataset, profile.selector);
      for (const key of Object.keys(filters)) {
        assert.ok(
          SAR_SEARCH_QUERY_KEYS.has(key),
          `资源 ${rid} 的 filters 键 '${key}' 必须属于 SarSearchQuery 权威字段集合`
        );
      }
    }
  });

  // 7. 未知资源拒绝并给出中文错误
  await t.test("7. Unknown resource profile is null and buildUnifiedSearchRequest rejects with Chinese error", () => {
    const profile = sarFilterProfile("science.unknown.dataset.v1");
    assert.equal(profile, null);

    const samplePolygon = {
      type: "Polygon",
      coordinates: [
        [
          [100, 20],
          [101, 20],
          [101, 21],
          [100, 21],
          [100, 20],
        ],
      ],
    };

    assert.throws(
      () => {
        buildUnifiedSearchRequest({
          resourceId: "science.unknown.dataset.v1",
          aoiGeometry: samplePolygon,
          aoiName: "测试区域",
        });
      },
      (err: any) => {
        return (
          err instanceof Error &&
          (err.message.includes("未知") || err.message.includes("不支持"))
        );
      },
      "未知数据源必须直接被拒绝并给出诚实中文错误"
    );
  });

  // 8. cleanStaleFilterValues: 切换数据源时剔除新 profile 不支持的过滤值
  await t.test("8. cleanStaleFilterValues drops unsupported values across profile switches", () => {
    const s1Profile = sarFilterProfile("science.sentinel-1.standard.v1");
    const alos2Profile = sarFilterProfile("science.alos-2.palsar-2.scansar-l1.1.v1");

    const s1Input: SarFilterInput = {
      platforms: ["Sentinel-1A"],
      productType: "GRD",
      relativeOrbit: 128,
      path: 128,
      orbitDirection: "ASCENDING",
      polarizations: ["VV"],
      frame: 400,
    };

    const cleanedForAlos2 = cleanStaleFilterValues(s1Profile, alos2Profile, s1Input);
    // ALOS-2 不支持 platforms -> []
    assert.deepEqual(cleanedForAlos2.platforms, []);
    // ALOS-2 不支持 GRD -> null
    assert.equal(cleanedForAlos2.productType, null);
    // ALOS-2 不支持 relative_orbit -> null
    assert.equal(cleanedForAlos2.relativeOrbit, null);
    // ALOS-2 支持 path -> 42/128 保留
    assert.equal(cleanedForAlos2.path, 128);
    // ALOS-2 极化仅支持 HH/HV，VV 必须被丢弃 -> []
    assert.deepEqual(cleanedForAlos2.polarizations, []);
    // ALOS-2 支持升降轨 -> 保留
    assert.equal(cleanedForAlos2.orbitDirection, "ASCENDING");
    // ALOS-2 支持 frame -> 保留
    assert.equal(cleanedForAlos2.frame, 400);
  });

  // 9. Relative orbit 与 Path 互斥：有 relativeOrbit 的源绝不暴露 path
  await t.test("10. Declarative FilterFieldSchema unifies SAR common/advanced tiers and hides pending_backend fields", () => {
    const s1 = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.ok(s1 !== null);
    const s1Common = visibleFilterFields(s1, "common").map((f) => f.id);
    const s1Advanced = visibleFilterFields(s1, "advanced").map((f) => f.id);
    assert.deepEqual(s1Common, ["productType", "polarizations", "orbitDirection"]);
    assert.ok(s1Advanced.includes("beamModes"), "Sentinel-1 advanced fields must include beamModes");
    assert.ok(s1Advanced.includes("platforms"), "Sentinel-1 advanced fields must include platforms");
    assert.equal(s1Advanced.includes("lookDirection"), false, "Sentinel-1 does not support lookDirection");
    assert.deepEqual(s1.beamModes?.options, ["IW", "EW", "SM", "WV"]);

    const burst = sarFilterProfile("science.sentinel-1.slc-burst.v1");
    assert.ok(burst !== null);
    assert.equal(visibleFilterFields(burst, "common").some((f) => f.id === "productType"), false, "Burst productType is fixed");
    assert.deepEqual(burst.beamModes?.options, ["IW", "EW"]);

    const nisar = sarFilterProfile("science.nisar.l.rslc.provisional.v1");
    assert.ok(nisar !== null);
    const pendingIds = nisar.fields.filter((f) => f.status === "pending_backend").map((f) => f.id);
    assert.deepEqual(pendingIds, ["lookDirection", "rangeBandwidth", "jointObservation", "frameCoverage"]);
    const visibleNisarIds = visibleFilterFields(nisar).map((f) => f.id);
    for (const id of pendingIds) {
      assert.equal(visibleNisarIds.includes(id), false, `pending_backend field ${id} must not be visible`);
    }
  });

  await t.test("11. Beam mode and look direction filters + ALOS-2 WBS polarization dependency", () => {
    const alos2 = sarFilterProfile("science.alos-2.palsar-2.scansar-l1.1.v1");
    assert.ok(alos2 !== null);
    assert.deepEqual(alos2.beamModes, { key: "beam_mode", options: ["WBS", "WBD"], multiple: false });
    assert.deepEqual(alos2.lookDirection, { key: "look_direction", options: ["RIGHT", "LEFT"], multiple: false });

    assert.deepEqual(allowedPolarizationsForBeamMode(alos2, ["WBS"]), ["HH"]);
    assert.deepEqual(allowedPolarizationsForBeamMode(alos2, ["WBD"]), ["HH", "HH+HV"]);

    const wbsFilters = buildSarFilters(alos2, {
      beamModes: ["WBS"],
      lookDirection: "right",
      polarizations: ["HH+HV"],
    });
    assert.equal(wbsFilters.beam_mode, "WBS");
    assert.equal(wbsFilters.look_direction, "RIGHT");
    assert.deepEqual(wbsFilters.polarizations, ["HH"], "WBS single-polarization must narrow HH+HV to HH");

    const alosPalsar = sarFilterProfile("science.alos.palsar.v1");
    assert.ok(alosPalsar !== null);
    const palsarFilters = buildSarFilters(alosPalsar, {
      beamModes: ["FBS", "FBD"],
      lookDirection: "LEFT",
    });
    assert.deepEqual(palsarFilters.beam_modes, ["FBS", "FBD"]);
    assert.equal(palsarFilters.look_direction, "LEFT");
  });

  await t.test("9. Relative orbit and Path are mutually exclusive per source", () => {
    // Sentinel-1: supports_relative_orbit=true 且 track_filter.parameter="relativeOrbit" -> 仅 relativeOrbit，无 path
    const s1Profile = sarFilterProfile("science.sentinel-1.standard.v1");
    assert.ok(s1Profile !== null);
    assert.ok(s1Profile.relativeOrbit !== null && s1Profile.relativeOrbit.key === "relative_orbit");
    assert.equal(s1Profile.path, null, "Sentinel-1 有 relativeOrbit 时严禁同时暴露 path");

    // ERS: 无 relative_orbit，仅有 path
    const ersProfile = sarFilterProfile("science.ers-1-2.sar.v1");
    assert.ok(ersProfile !== null);
    assert.equal(ersProfile.relativeOrbit, null, "ERS 无 relative_orbit");
    assert.ok(ersProfile.path !== null && ersProfile.path.key === "path", "ERS 必须暴露 path");

    // ALOS PALSAR: 无 relative_orbit，仅有 path
    const alosProfile = sarFilterProfile("science.alos.palsar.v1");
    assert.ok(alosProfile !== null);
    assert.equal(alosProfile.relativeOrbit, null);
    assert.ok(alosProfile.path !== null && alosProfile.path.key === "path");
  });
});
