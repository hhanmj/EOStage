import test from "node:test";
import assert from "node:assert/strict";
import { buildUnifiedSearchRequest } from "./searchRequest.ts";
import { sarFilterProfile } from "./sarFilterProfile.ts";

const M2_ALL_7_RESOURCES = [
  "science.opera.disp-s1.v1",
  "science.comet.licsar.v1",
  "dem.aster-gdem.v3",
  "event.usgs.earthquake.v1",
  "geodesy.ngl.gnss.v1",
  "atmospheric.era5.reanalysis.v1",
  "ionospheric.igs.gim.v1",
] as const;

const AUXILIARY_5_RESOURCES = [
  "dem.aster-gdem.v3",
  "event.usgs.earthquake.v1",
  "geodesy.ngl.gnss.v1",
  "atmospheric.era5.reanalysis.v1",
  "ionospheric.igs.gim.v1",
] as const;

const SAR_2_RESOURCES = [
  "science.opera.disp-s1.v1",
  "science.comet.licsar.v1",
] as const;

const VALID_AOI = {
  type: "Polygon",
  coordinates: [
    [
      [110.0, 30.0],
      [111.0, 30.0],
      [111.0, 31.0],
      [110.0, 31.0],
      [110.0, 30.0],
    ],
  ],
};

test("Challenger M2: Frontend buildUnifiedSearchRequest Stress Harness", async (t) => {
  // 1. All 7 resources build cleanly with valid parameters and paired dates
  await t.test("1. All 7 resources build valid UnifiedSearchRequest with normal parameters", () => {
    for (const resourceId of M2_ALL_7_RESOURCES) {
      const result = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "标准测试区",
        startDate: "2024-01-01",
        endDate: "2024-01-10",
      });

      assert.equal(result.resource_id, resourceId);
      assert.deepEqual(result.aoi, VALID_AOI);
      assert.equal(result.aoi_name, "标准测试区");
      assert.ok(result.request_id.startsWith("req-"));
      assert.deepEqual(result.time_range, {
        start: "2024-01-01T00:00:00Z",
        end: "2024-01-10T23:59:59Z",
      });
      assert.ok(typeof result.filters === "object");
    }
  });

  // 2. All 7 resources build cleanly with no dates (time_range: null)
  await t.test("2. All 7 resources build cleanly with no dates", () => {
    for (const resourceId of M2_ALL_7_RESOURCES) {
      const result = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "无日期测试区",
      });

      assert.equal(result.resource_id, resourceId);
      assert.equal(result.time_range, null);
      assert.equal(result.aoi_name, "无日期测试区");
      assert.ok(result.request_id.length > 5);
    }
  });

  // 3. Missing or empty AOI throws missingAoiError for all 7 resources
  await t.test("3. Missing, null, or empty AOI geometry throws missingAoiError", () => {
    for (const resourceId of M2_ALL_7_RESOURCES) {
      for (const badAoi of [null, undefined, {}]) {
        assert.throws(
          () =>
            buildUnifiedSearchRequest({
              resourceId,
              aoiGeometry: badAoi as any,
              aoiName: "测试区",
            }),
          /请先设定 AOI/
        );
      }
    }
  });

  // 4. Date validation edge cases: unpaired dates, inverted dates
  await t.test("4. Date validation rejects single dates and inverted dates", () => {
    for (const resourceId of M2_ALL_7_RESOURCES) {
      // Single start date only
      assert.throws(
        () =>
          buildUnifiedSearchRequest({
            resourceId,
            aoiGeometry: VALID_AOI,
            aoiName: "单开始日期",
            startDate: "2024-01-01",
          }),
        /开始和结束日期需同时填写/
      );

      // Single end date only
      assert.throws(
        () =>
          buildUnifiedSearchRequest({
            resourceId,
            aoiGeometry: VALID_AOI,
            aoiName: "单结束日期",
            endDate: "2024-01-10",
          }),
        /开始和结束日期需同时填写/
      );

      // Inverted dates (start > end)
      assert.throws(
        () =>
          buildUnifiedSearchRequest({
            resourceId,
            aoiGeometry: VALID_AOI,
            aoiName: "倒序日期",
            startDate: "2024-05-10",
            endDate: "2024-01-01",
          }),
        /结束日期不能早于开始日期/
      );
    }
  });

  // 5. AOI Name whitespace fallback and Unicode preservation
  await t.test("5. AOI Name trims whitespace or falls back to default name", () => {
    for (const resourceId of M2_ALL_7_RESOURCES) {
      // Empty string -> fallback
      const emptyRes = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "",
      });
      assert.equal(emptyRes.aoi_name, "地图选定范围");

      // Whitespace only -> fallback
      const spaceRes = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "   \t\n  ",
      });
      assert.equal(spaceRes.aoi_name, "地图选定范围");

      // Unicode / emoji -> preserved
      const unicodeRes = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "  🛰️ 西藏那曲监测区 2026  ",
      });
      assert.equal(unicodeRes.aoi_name, "🛰️ 西藏那曲监测区 2026");
    }
  });

  // 6. Auxiliary 5 resources strictly emit empty filters object
  await t.test("6. Auxiliary resources emit empty filters object without unneeded parameters", () => {
    for (const resourceId of AUXILIARY_5_RESOURCES) {
      const result = buildUnifiedSearchRequest({
        resourceId,
        aoiGeometry: VALID_AOI,
        aoiName: "辅助数据源",
        platforms: ["SENTINEL-1A"],
        productType: "SLC",
        polarizations: ["VV"],
        beamModes: ["IW"],
        orbitDirection: "ascending",
        frame: 100,
        path: 200,
        relativeOrbit: 300,
      });

      assert.deepEqual(
        result.filters,
        {},
        `Auxiliary resource ${resourceId} must have strictly empty filters`
      );
    }
  });

  // 7. SAR 2 resources (OPERA and LiCSAR) emit typed filters conforming to contracts
  await t.test("7. SAR resources emit typed filters conforming to registry contracts", () => {
    // 7a. OPERA S1
    const operaProfile = sarFilterProfile("science.opera.disp-s1.v1");
    assert.ok(operaProfile !== null);
    assert.equal(operaProfile.kind, "typed");
    assert.equal(operaProfile.selector, "OPERA-S1");

    const operaRes = buildUnifiedSearchRequest({
      resourceId: "science.opera.disp-s1.v1",
      aoiGeometry: VALID_AOI,
      aoiName: "OPERA测试",
      orbitDirection: "ascending",
      polarizations: ["VV", "VH"],
      beamModes: ["IW"],
      productType: "DISP-S1",
    });

    assert.equal("dataset" in operaRes.filters, false, "Typed SAR must not emit dataset");
    assert.equal("processing_levels" in operaRes.filters, false, "Typed SAR must not emit processing_levels");
    assert.equal("platforms" in operaRes.filters, false, "Typed SAR must not emit platforms");
    assert.equal(operaRes.filters.flight_direction, "ASCENDING");

    // 7b. COMET LiCSAR
    const licsarProfile = sarFilterProfile("science.comet.licsar.v1");
    assert.ok(licsarProfile !== null);
    assert.equal(licsarProfile.kind, "typed");
    assert.equal(licsarProfile.selector, "LICSAR");

    const licsarRes = buildUnifiedSearchRequest({
      resourceId: "science.comet.licsar.v1",
      aoiGeometry: VALID_AOI,
      aoiName: "LiCSAR测试",
      frame: 1024,
      orbitDirection: "descending",
    });

    assert.equal("dataset" in licsarRes.filters, false);
    assert.equal(licsarRes.filters.frame, 1024);
    // flight_direction is not in LiCSAR supported_keys, so it must NOT be emitted
    assert.equal("flight_direction" in licsarRes.filters, false);
  });

  // 8. Truly unsupported resource throws unsupportedResourceError
  await t.test("8. Truly unknown/unsupported resource throws unsupportedResourceError", () => {
    assert.throws(
      () =>
        buildUnifiedSearchRequest({
          resourceId: "unknown.fictional.dataset.v999",
          aoiGeometry: VALID_AOI,
          aoiName: "未知数据源",
        }),
      /不支持或未知的数据源/
    );
  });
});
