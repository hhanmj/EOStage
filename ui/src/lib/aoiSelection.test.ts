import assert from "node:assert/strict";
import test from "node:test";

import {
  aoiBboxInputValues,
  beginLatestRequest,
  boundaryToActualAoi,
  createAoiSelectionState,
  findExactSelectorBoundary,
  chinaSelectorPresentation,
  isDirectControlledMunicipality,
  isTaiwanSelectorProvince,
  isLatestResponse,
  planChinaSelectorBoundaryResult,
  reduceAoiSelection,
  reduceChinaSelector,
  resolveActualAoi,
  selectedChinaProviderObjectId,
  selectorBoundaryTarget,
  stableBoundaryIdentity,
  type BoundaryCandidate,
} from "./aoiSelection.ts";

const bbox = { west: 100, east: 101, south: 24, north: 25, crs: "EPSG:4326" };

function boundary(
  code: string,
  name: string,
  level: "country" | "province" | "city" | "district",
): BoundaryCandidate {
  return {
    label: name,
    name,
    display_name: name,
    boundary_id: `datav:${code}`,
    provider: "datav",
    provider_object_id: code,
    admin_level: level,
    geometry: {
      type: "Polygon",
      coordinates: [[[100, 24], [101, 24], [101, 25], [100, 24]]],
    },
    bbox,
    source: "DataV areas_v3",
  };
}

const yunnan = boundary("530000", "云南省", "province");
const kunming = boundary("530100", "昆明市", "city");
const wuhua = boundary("530102", "五华区", "district");
const chengdu = boundary("510100", "成都市", "city");
const china = boundary("100000", "中国", "country");

test("China selector chooses the exact selected-level DataV identity", () => {
  const identities = {
    province: "650000",
    city: "659000",
    district: "659012",
  };
  assert.equal(
    selectedChinaProviderObjectId(
      { province: "DataV测试省", city: "DataV测试市", district: "DataV测试新区" },
      identities,
    ),
    "659012",
  );
  assert.equal(
    selectedChinaProviderObjectId(
      { province: "DataV测试省", city: "DataV测试市", district: "全部" },
      identities,
    ),
    "659000",
  );
  assert.equal(
    selectedChinaProviderObjectId(
      { province: "DataV测试省", city: "全部", district: "全部" },
      identities,
    ),
    "650000",
  );
});

test("legacy fallback option without identity never borrows a parent adcode", () => {
  assert.equal(
    selectedChinaProviderObjectId(
      { province: "DataV测试省", city: "DataV测试市", district: "Legacy测试县" },
      { province: "650000", city: "659000", district: null },
    ),
    null,
  );
});

test("AOI-00 national selection binds the exact national authority", () => {
  const target = selectorBoundaryTarget({ province: "全国", city: "全部", district: "全部" });

  assert.deepEqual(target, {
    province: "全国",
    city: "全部",
    district: "全部",
    expectedName: "中国",
    expectedLevel: "country",
  });
  assert.equal(findExactSelectorBoundary([china], target), china);
});

test("AOI-01 province selection binds the exact province instead of the first child", () => {
  const target = selectorBoundaryTarget({ province: "云南省", city: "全部", district: "全部" });

  assert.deepEqual(target, {
    province: "云南省",
    city: "全部",
    district: "全部",
    expectedName: "云南省",
    expectedLevel: "province",
  });
  assert.equal(findExactSelectorBoundary([kunming, yunnan], target), yunnan);
});

test("AOI-02 city selection binds the exact city instead of the first district", () => {
  const target = selectorBoundaryTarget({ province: "云南省", city: "昆明市", district: "全部" });

  assert.equal(target?.expectedName, "昆明市");
  assert.equal(target?.expectedLevel, "city");
  assert.equal(findExactSelectorBoundary([wuhua, kunming], target), kunming);
});

test("the four exact municipality identities project selector two as district and disable selector three", () => {
  for (const adcode of ["110000", "120000", "310000", "500000"]) {
    assert.equal(isDirectControlledMunicipality(adcode), true, adcode);
    assert.deepEqual(chinaSelectorPresentation(adcode), {
      municipality: true,
      twoLevel: true,
      secondLevel: "district",
      showThirdSelector: false,
    });
  }
  assert.equal(isDirectControlledMunicipality("420000"), false);
  assert.deepEqual(chinaSelectorPresentation("420000"), {
    municipality: false,
    twoLevel: false,
    secondLevel: "city",
    showThirdSelector: true,
  });
});

test("Taiwan is a province-owned two-level selector and never a municipality", () => {
  assert.equal(isDirectControlledMunicipality("710000"), false);
  assert.equal(isTaiwanSelectorProvince("710000", "台湾省"), true);
  assert.deepEqual(chinaSelectorPresentation("710000"), {
    municipality: false,
    twoLevel: true,
    secondLevel: "city",
    showThirdSelector: false,
  });

  const state = { province: "台湾省", city: "南投县", district: "全部" };
  const identities = { province: "710000", city: "710800", district: null };
  assert.equal(selectedChinaProviderObjectId(state, identities), "710800");
  assert.equal(selectedChinaProviderObjectId({ province: "台湾省", city: "全部", district: "全部" }, { province: "710000", city: null, district: null }), "710000");
  assert.equal(
    selectedChinaProviderObjectId(state, { province: "710000", city: "156710800", district: null }),
    "156710800",
  );
  // Foreign OSM relation:* identities must never be accepted for Taiwan
  assert.equal(
    selectedChinaProviderObjectId(state, { province: "relation:449220", city: "relation:2497975", district: null }),
    null,
  );
  assert.equal(selectorBoundaryTarget(state, identities)?.expectedLevel, "city");
});

test("Hong Kong and Macao are DataV direct-child two-level selectors, not municipalities", () => {
  for (const [province, provinceAdcode, child, childAdcode] of [
    ["香港特别行政区", "810000", "湾仔区", "810002"],
    ["澳门特别行政区", "820000", "花地玛堂区", "820001"],
  ] as const) {
    assert.equal(isDirectControlledMunicipality(provinceAdcode), false, province);
    assert.deepEqual(chinaSelectorPresentation(provinceAdcode, province), {
      municipality: false,
      twoLevel: true,
      secondLevel: "district",
      showThirdSelector: false,
    });

    const state = { province, city: child, district: "全部" };
    const identities = { province: provinceAdcode, city: childAdcode, district: null };
    assert.equal(selectedChinaProviderObjectId(state, identities), childAdcode);
    assert.deepEqual(selectorBoundaryTarget(state, identities), {
      ...state,
      expectedName: child,
      expectedLevel: "district",
    });
  }
});

test("two-level presentation remains stable when fallback options have display identity only", () => {
  assert.deepEqual(chinaSelectorPresentation(null, "台湾省"), {
    municipality: false,
    twoLevel: true,
    secondLevel: "city",
    showThirdSelector: false,
  });
  assert.deepEqual(chinaSelectorPresentation(null, "北京市"), {
    municipality: true,
    twoLevel: true,
    secondLevel: "district",
    showThirdSelector: false,
  });
  assert.deepEqual(chinaSelectorPresentation(null, "香港特别行政区"), {
    municipality: false,
    twoLevel: true,
    secondLevel: "district",
    showThirdSelector: false,
  });
  assert.deepEqual(chinaSelectorPresentation(null, "澳门特别行政区"), {
    municipality: false,
    twoLevel: true,
    secondLevel: "district",
    showThirdSelector: false,
  });
  assert.deepEqual(chinaSelectorPresentation(null, "湖北省"), {
    municipality: false,
    twoLevel: false,
    secondLevel: "city",
    showThirdSelector: true,
  });
});

test("Beijing selector two binds Haidian as the exact district identity", () => {
  const state = { province: "北京市", city: "海淀区", district: "全部" };
  const identities = { province: "110000", city: "110108", district: null };
  const target = selectorBoundaryTarget(state, identities);
  const haidian = boundary("110108", "海淀区", "district");

  assert.equal(selectedChinaProviderObjectId(state, identities), "110108");
  assert.equal(target?.expectedName, "海淀区");
  assert.equal(target?.expectedLevel, "district");
  assert.equal(findExactSelectorBoundary([boundary("110000", "北京市", "province"), haidian], target), haidian);
});

test("China deterministic DataV result is selected for immediate working-AOI binding", () => {
  const target = selectorBoundaryTarget({ province: "云南省", city: "全部", district: "全部" });
  assert.deepEqual(
    planChinaSelectorBoundaryResult("datav", [kunming, yunnan], target),
    { kind: "bind", boundary: yunnan },
  );
});

test("China OSM fallback preserves multiple candidates for explicit user selection", () => {
  const candidates: BoundaryCandidate[] = [
    {
      label: "巴东县候选一",
      provider: "nominatim",
      provider_object_id: "relation:41",
      admin_level: "administrative",
      bbox,
    },
    {
      label: "巴东县候选二",
      provider: "nominatim",
      provider_object_id: "relation:42",
      admin_level: "administrative",
      bbox,
    },
  ];
  const target = selectorBoundaryTarget({
    province: "湖北省",
    city: "恩施土家族苗族自治州",
    district: "巴东县",
  });
  assert.deepEqual(
    planChinaSelectorBoundaryResult("nominatim", candidates, target),
    { kind: "choose_candidates", candidates },
  );
});

test("Taiwan selector directly binds its domestic DataV/Tianditu polygon while rejecting Nominatim in Taiwan route", () => {
  const taiwanDomesticPolygon: BoundaryCandidate = {
    label: "台湾省 · 南投县",
    name: "南投县",
    display_name: "台湾省 南投县",
    provider: "tianditu",
    provider_object_id: "710800",
    admin_level: "city",
    geometry: {
      type: "Polygon",
      coordinates: [[[120.5, 23.5], [121, 23.5], [121, 24], [120.5, 23.5]]],
    },
    bbox,
  };
  const taiwanForeignOsmPolygon: BoundaryCandidate = {
    ...taiwanDomesticPolygon,
    provider: "nominatim",
    provider_object_id: "relation:2497975",
    admin_level: "administrative",
  };
  const target = selectorBoundaryTarget({ province: "台湾省", city: "南投县", district: "全部" });

  assert.deepEqual(
    planChinaSelectorBoundaryResult("tianditu", [taiwanDomesticPolygon], target, {
      directResolvedSelector: true,
    }),
    { kind: "bind", boundary: taiwanDomesticPolygon },
  );
  assert.deepEqual(
    planChinaSelectorBoundaryResult("nominatim", [taiwanForeignOsmPolygon], target, {
      directResolvedSelector: true,
    }),
    { kind: "not_found" },
  );
  assert.deepEqual(
    planChinaSelectorBoundaryResult("nominatim", [taiwanForeignOsmPolygon], target),
    { kind: "choose_candidates", candidates: [taiwanForeignOsmPolygon] },
  );
});

test("AOI-03 district selection binds the exact selected district", () => {
  const target = selectorBoundaryTarget({ province: "云南省", city: "昆明市", district: "五华区" });

  assert.equal(target?.expectedName, "五华区");
  assert.equal(target?.expectedLevel, "district");
  assert.equal(findExactSelectorBoundary([kunming, wuhua], target), wuhua);
});

test("AOI-03b selector accepts a bbox-only exact candidate for later resolve", () => {
  const bboxOnly = { ...yunnan, geometry: undefined };
  const target = selectorBoundaryTarget({ province: "云南省", city: "全部", district: "全部" });

  assert.equal(findExactSelectorBoundary([bboxOnly], target), bboxOnly);
  assert.equal(boundaryToActualAoi(bboxOnly), null);
});

test("AOI-04 selector state stays independent when a global result is selected and bound", () => {
  let state = createAoiSelectionState({
    chinaSelector: { province: "云南省", city: "昆明市", district: "全部" },
  });
  state = reduceAoiSelection(state, { type: "global/query", query: "成都" });
  state = reduceAoiSelection(state, { type: "global/results", results: [chengdu] });
  state = reduceAoiSelection(state, {
    type: "global/select",
    identity: stableBoundaryIdentity(chengdu),
  });
  state = reduceAoiSelection(state, { type: "bind/selected" });

  assert.deepEqual(state.chinaSelector, {
    province: "云南省",
    city: "昆明市",
    district: "全部",
  });
  assert.equal(state.workingAoi?.displayName, "成都市");
  assert.equal(state.workingAoi?.identity, "datav:510100");
});

test("AOI-05 selecting result N preserves that stable identity through preview and bind", () => {
  const results = Array.from({ length: 10 }, (_, index) =>
    boundary(`fixture-${index}`, `Springfield ${index}`, "city"));
  let state = createAoiSelectionState();
  state = reduceAoiSelection(state, { type: "global/results", results });
  state = reduceAoiSelection(state, {
    type: "global/select",
    identity: stableBoundaryIdentity(results[6]),
  });

  assert.equal(state.previewBoundary, results[6]);
  assert.equal(state.workingAoi, null, "preview must not bind");

  state = reduceAoiSelection(state, { type: "bind/selected" });
  assert.equal(state.workingAoi?.identity, stableBoundaryIdentity(results[6]));
  assert.notEqual(state.workingAoi?.identity, stableBoundaryIdentity(results[0]));
});

test("AOI-06 parent changes synchronously clear stale child selections", () => {
  const initial = { province: "云南省", city: "昆明市", district: "五华区" };

  assert.deepEqual(reduceChinaSelector(initial, { field: "province", value: "四川省" }), {
    province: "四川省",
    city: "全部",
    district: "全部",
  });
  assert.deepEqual(reduceChinaSelector(initial, { field: "city", value: "曲靖市" }), {
    province: "云南省",
    city: "曲靖市",
    district: "全部",
  });
});

test("AOI-07 preview never becomes the bound or working AOI implicitly", () => {
  let state = createAoiSelectionState();
  state = reduceAoiSelection(state, { type: "preview/boundary", boundary: chengdu });

  assert.equal(state.previewBoundary, chengdu);
  assert.equal(state.selectedSearchResult, null);
  assert.equal(state.workingAoi, null);
});

test("AOI-08 a persisted standalone AOI resolves without Workspace or Region state", () => {
  const working = {
    identity: "working:drawn",
    source: "drawn",
    level: null,
    displayName: "绘制范围",
    geometry: yunnan.geometry!,
    bbox,
  };

  assert.equal(resolveActualAoi({
    regionSelected: false,
    workingAoi: working,
    persistedRegionAoi: null,
  }), working);
});

test("AOI-09 selected Region owns active AOI while standalone remains dormant", () => {
  const persisted = {
    identity: "region:old",
    source: "region",
    level: null,
    displayName: "旧研究区",
    geometry: yunnan.geometry!,
    bbox,
  };
  const working = {
    identity: "datav:510100",
    source: "datav",
    level: "city",
    displayName: "成都市",
    geometry: chengdu.geometry!,
    bbox,
  };

  assert.equal(
    resolveActualAoi({
      regionSelected: true,
      workingAoi: working,
      persistedRegionAoi: persisted,
      previewBoundary: wuhua,
    }),
    persisted,
  );
  assert.equal(resolveActualAoi({
    regionSelected: true,
    workingAoi: working,
    persistedRegionAoi: null,
  }), null, "selected Region without an AOI must not leak the dormant standalone AOI");
  assert.equal(resolveActualAoi({
    regionSelected: false,
    workingAoi: working,
    persistedRegionAoi: persisted,
  }), working, "leaving Region reactivates the standalone AOI");
});

test("AOI-09b an authoritative boundary bbox is copied exactly into manual AOI inputs", () => {
  assert.deepEqual(aoiBboxInputValues({
    west: 113.6965,
    east: 115.0768,
    south: 29.9718,
    north: 31.3633,
    crs: "EPSG:4326",
  }), {
    west: "113.6965",
    east: "115.0768",
    south: "29.9718",
    north: "31.3633",
  });
});

test("AOI-10 a late administrative response cannot overwrite the latest request", () => {
  const first = beginLatestRequest(0);
  const second = beginLatestRequest(first.activeGeneration);

  assert.equal(isLatestResponse(second.activeGeneration, first.requestGeneration), false);
  assert.equal(isLatestResponse(second.activeGeneration, second.requestGeneration), true);
});
