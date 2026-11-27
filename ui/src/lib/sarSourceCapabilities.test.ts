import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import test from "node:test";
import type {
  DatasetCapabilities,
  SarSourceCapability,
} from "./generated/v3-contracts.ts";
import {
  buildTypedSarCatalogQuery,
  datasetCapabilityForSarSource,
  nisarSearchSourceAdmission,
  nisarSourceForBand,
  nisarSubsetCapabilityMessage,
  sarSourceChoiceValues,
  sarSourceOptionValues,
} from "./sarSourceCapabilities.ts";

const fixture = JSON.parse(
  readFileSync(new URL("../../../contracts/v3/asf-capabilities.v1.json", import.meta.url), "utf8"),
) as {
  datasets: DatasetCapabilities[];
  sources: SarSourceCapability[];
};

const source = (selector: string) => {
  const value = fixture.sources.find((item) => item.selector === selector);
  assert.ok(value, `missing fixture source ${selector}`);
  if (value.search_kind !== "legacy_asf_search") return value;
  const dataset = fixture.datasets.find((item) => item.dataset === selector);
  assert.ok(dataset, `missing fixture DatasetCapabilities ${selector}`);
  return {
    ...value,
    filters: { ...value.filters, dataset_capabilities: dataset },
  };
};

const aoi = { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]] };

test("ALOS PALSAR remains ALOS-1 legacy while ALOS-2 is a distinct typed source", () => {
  const legacy = source("ALOS PALSAR");
  const alos2 = source("ALOS-2");
  const legacyDataset = fixture.datasets.find((item) => item.dataset === "ALOS PALSAR");

  assert.equal(legacy.display_name, "ALOS PALSAR");
  assert.equal(legacy.search_kind, "legacy_asf_search");
  assert.equal(legacy.filters.legacy_dataset, "ALOS PALSAR");
  assert.equal(legacyDataset?.platforms[0], "ALOS");
  assert.notEqual(legacy.selector, alos2.selector);
  assert.equal(alos2.display_name, "ALOS-2 PALSAR-2 ScanSAR L1.1");
  assert.equal(alos2.logical_dataset, "sar.alos-2.palsar-2.scansar.l1.1.v1");
  assert.equal(alos2.provider_id, "asf.nasa.earthdata");
  assert.equal(alos2.native_collection_id, "C3315903479-ASF");
  assert.equal(alos2.collection_profile_id, "asf.alos-2.palsar-2.scansar.l1.1.v1");
  assert.deepEqual(sarSourceOptionValues(alos2, "beam_mode"), ["WBS", "WBD"]);
  assert.equal(alos2.filters.fixed.processing_level, "1.1");
});

test("NISAR exposes the reviewed collection/profile identity and fixed product values", () => {
  const nisar = source("NISAR");
  assert.equal(nisar.display_name, "NISAR L-band RSLC (Provisional)");
  assert.equal(nisar.search_kind, "typed_catalog_profile");
  assert.equal(nisar.logical_dataset, "sar.nisar.l.rslc.provisional.v1");
  assert.equal(nisar.provider_id, "asf.nasa.earthdata");
  assert.equal(nisar.native_collection_id, "C2853145197-ASF");
  assert.equal(nisar.collection_profile_id, "asf.nisar.l.rslc.provisional.v1");
  assert.equal(nisar.filters.fixed.product_type, "RSLC");
  assert.equal(nisar.filters.fixed.production_configuration, "PR");
  assert.equal(nisar.filters.fixed.crid, "P05023");
  assert.equal(nisar.filters.fixed.product_specification_version, "1.5.0");
});

test("typed ALOS-2/NISAR query construction never creates a generic ASF dataset query", () => {
  const alos2 = { ...source("ALOS-2"), available: true, disabled_reason: null };
  const nisar = { ...source("NISAR"), available: true, disabled_reason: null };
  const alos2Query = buildTypedSarCatalogQuery({
    source: alos2,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit: 24,
    alos2BeamMode: "WBD",
    alos2FlightDirection: "DESCENDING",
    alos2LookDirection: "RIGHT",
  });
  const nisarQuery = buildTypedSarCatalogQuery({
    source: nisar,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit: 24,
    nisarFrameCoverage: "FULL",
    nisarFrequencyA: "HH",
    nisarFrequencyB: "VV",
    nisarRangeBandwidth: "5",
  });

  assert.equal(alos2Query.ok, true);
  assert.equal(nisarQuery.ok, true);
  if (!alos2Query.ok || !nisarQuery.ok) return;
  assert.equal(alos2Query.query.logical_dataset, "sar.alos-2.palsar-2.scansar.l1.1.v1");
  assert.equal(nisarQuery.query.logical_dataset, "sar.nisar.l.rslc.provisional.v1");
  assert.equal(alos2Query.query.provider_id, "asf.nasa.earthdata");
  assert.equal(nisarQuery.query.native_collection_id, "C2853145197-ASF");
  assert.equal("dataset" in alos2Query.query, false);
  assert.equal("dataset" in nisarQuery.query, false);
  assert.equal(alos2Query.query.filters.processing_level, undefined);
  assert.equal(nisarQuery.query.filters.product_type, undefined);
  assert.equal(nisarQuery.query.filters.production_configuration, "PR");
});

test("typed catalogue query carries user result intent and never a provider page size", () => {
  const nisar = { ...source("NISAR"), available: true, disabled_reason: null };
  const build = (resultLimit: number | null) => buildTypedSarCatalogQuery({
    source: nisar,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit,
  });

  for (const resultLimit of [1, 17, 250, 251, 617, 3000, 10_000, null]) {
    const result = build(resultLimit);
    assert.equal(result.ok, true);
    if (!result.ok) continue;
    const query = result.query as unknown as Record<string, unknown>;
    assert.equal(query.result_limit, resultLimit);
    assert.equal("page_size" in query, false);
    if (resultLimit === null) assert.notEqual(query.result_limit, 10_000);
  }
});

test("route-unavailable typed sources remain visible and fail closed without fallback", () => {
  const alos2 = {
    ...source("ALOS-2"),
    available: false,
    disabled_reason: "CATALOGUE_ROUTE_UNAVAILABLE",
  };
  const result = buildTypedSarCatalogQuery({
    source: alos2,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit: 24,
  });

  assert.equal(alos2.available, false);
  assert.equal(alos2.disabled_reason, "CATALOGUE_ROUTE_UNAVAILABLE");
  assert.equal(result.ok, false);
  if (result.ok) return;
  assert.match(result.error, /当前不可用/);
  assert.match(result.error, /CATALOGUE_ROUTE_UNAVAILABLE/);
});

test("fixture-enabled typed construction is pure offline work and preserves fixed NISAR constraints", () => {
  const nisar = { ...source("NISAR"), available: true, disabled_reason: null };
  const result = buildTypedSarCatalogQuery({
    source: nisar,
    aoiGeojson: aoi,
    startUtc: "2026-01-01T00:00:00Z",
    endUtc: "2026-01-02T00:00:00Z",
    resultLimit: 24,
    nisarFrameCoverage: "PARTIAL",
    nisarFrequencyA: "HH+HV",
    nisarFrequencyB: "VV",
    nisarRangeBandwidth: "77",
  });

  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(result.query.filters.datetime, {
    start: "2026-01-01T00:00:00Z",
    end: "2026-01-02T00:00:00Z",
  });
  assert.equal(result.query.filters.production_configuration, "PR");
  assert.equal(result.query.filters.frame_coverage, "PARTIAL");
  assert.equal(result.query.filters.frequency_a_polarization, "HH+HV");
  assert.equal(result.query.filters.frequency_b_polarization, "VV");
  assert.equal(result.query.filters.range_bandwidth, "77");
});

test("blank NISAR optional filters mean all values and do not synthesize the first registry option", () => {
  const nisar = { ...source("NISAR"), available: true, disabled_reason: null };
  const result = buildTypedSarCatalogQuery({
    source: nisar,
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "",
    resultLimit: 250,
    nisarFrameCoverage: "",
    nisarFrequencyA: "",
    nisarFrequencyB: "",
    nisarRangeBandwidth: "",
    nisarFlightDirection: "",
    nisarLookDirection: "",
  });

  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.query.filters.production_configuration, "PR");
  for (const key of [
    "frame_coverage",
    "frequency_a_polarization",
    "frequency_b_polarization",
    "range_bandwidth",
    "flight_direction",
    "look_direction",
  ]) {
    assert.equal(result.query.filters[key], undefined, key);
  }
});

test("typed catalogue rejects a finite result limit above the contract maximum", () => {
  const nisar = { ...source("NISAR"), available: true, disabled_reason: null };
  const result = buildTypedSarCatalogQuery({
    source: nisar,
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "",
    resultLimit: 10_001,
  });

  assert.equal(result.ok, false);
  if (!result.ok) assert.match(result.error, /Result Limit/);
});

test("all historical ASF sources derive a total DatasetCapabilities projection from their own source authority", () => {
  for (const selector of ["ALOS PALSAR", "SEASAT"]) {
    const projected = datasetCapabilityForSarSource(source(selector));
    assert.equal(projected.ok, true, selector);
    if (!projected.ok) continue;
    assert.equal(projected.capability.dataset, selector);
    assert.ok(projected.capability.platforms.length > 0, selector);
    if (selector === "SEASAT") {
      assert.deepEqual(projected.capability.processing_levels, [], selector);
      assert.deepEqual(projected.capability.product_families, ["OTHER"], selector);
    } else {
      assert.ok(projected.capability.processing_levels.length > 0, selector);
    }
  }
});

test("typed product choices derive from the registry and reject a future selection without an executable profile", () => {
  const nisar = source("NISAR");
  const expanded = {
    ...nisar,
    filters: {
      ...nisar.filters,
      options: { ...nisar.filters.options, product_type: ["RSLC", "GSLC"] },
    },
  };
  const choices = sarSourceChoiceValues(expanded, "product_type");
  assert.deepEqual(choices, ["RSLC", "GSLC"]);

  const current = buildTypedSarCatalogQuery({
    source: expanded,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit: 24,
    nisarProductType: choices[0],
    nisarBand: "L",
    nisarMaturity: "Provisional",
    nisarFrameCoverage: "FULL",
    nisarFrequencyA: "HH",
    nisarFrequencyB: "HH",
    nisarRangeBandwidth: "5",
  });
  assert.equal(current.ok, true);

  const unsupported = buildTypedSarCatalogQuery({
    source: expanded,
    aoiGeojson: aoi,
    startUtc: "2026-01-01",
    endUtc: "2026-01-02",
    resultLimit: 24,
    nisarProductType: choices[1],
    nisarBand: "L",
    nisarMaturity: "Provisional",
    nisarFrameCoverage: "FULL",
    nisarFrequencyA: "HH",
    nisarFrequencyB: "HH",
    nisarRangeBandwidth: "5",
  });
  assert.equal(unsupported.ok, false);
  if (!unsupported.ok) assert.match(unsupported.error, /尚未映射到可执行/);
});

test("typed catalogue dates support unbounded and one-sided intervals", () => {
  const alos2 = { ...source("ALOS-2"), available: true, disabled_reason: null };
  const build = (startUtc: string, endUtc: string) => buildTypedSarCatalogQuery({
    source: alos2,
    aoiGeojson: aoi,
    startUtc,
    endUtc,
    resultLimit: 24,
  });

  const unbounded = build("", "");
  const startOnly = build("2026-01-01", "");
  const endOnly = build("", "2026-01-31");
  const sameDay = build("2026-01-15", "2026-01-15");

  assert.equal(unbounded.ok, true);
  assert.equal(startOnly.ok, true);
  assert.equal(endOnly.ok, true);
  assert.equal(sameDay.ok, true);
  if (!unbounded.ok || !startOnly.ok || !endOnly.ok || !sameDay.ok) return;
  assert.equal(unbounded.query.filters.datetime, undefined);
  assert.deepEqual(startOnly.query.filters.datetime, { start: "2026-01-01T00:00:00Z" });
  assert.deepEqual(endOnly.query.filters.datetime, { end: "2026-01-31T23:59:59Z" });
  assert.deepEqual(sameDay.query.filters.datetime, {
    start: "2026-01-15T00:00:00Z",
    end: "2026-01-15T23:59:59Z",
  });
});

test("typed catalogue dates reject only malformed values and start after end", () => {
  const alos2 = { ...source("ALOS-2"), available: true, disabled_reason: null };
  const build = (startUtc: string, endUtc: string) => buildTypedSarCatalogQuery({
    source: alos2,
    aoiGeojson: aoi,
    startUtc,
    endUtc,
    resultLimit: 24,
  });

  const malformed = build("2026-02-30", "");
  const reversed = build("2026-02-02", "2026-02-01");
  assert.equal(malformed.ok, false);
  assert.equal(reversed.ok, false);
  if (malformed.ok || reversed.ok) return;
  assert.match(malformed.error, /开始日期.*格式无效/);
  assert.match(reversed.error, /不能晚于/);
});

test("blank ALOS-2 Beam means all modes and does not synthesize polarization", () => {
  const alos2 = { ...source("ALOS-2"), available: true, disabled_reason: null };
  const result = buildTypedSarCatalogQuery({
    source: alos2,
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "",
    resultLimit: 24,
    alos2BeamMode: "",
  });

  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.query.filters.beam_mode, undefined);
  assert.equal(result.query.filters.polarizations, undefined);
});

test("the ASF-removed legacy missions are absent from the compatibility contract", () => {
  // ASF removed ERS, JERS-1 and RADARSAT-1 from its searchable interfaces
  // (Owner evidence 2026-10-31): they stay in the registry as retired
  // identities but are no longer projected as ASF sources.
  const selectors = fixture.sources.map((item) => item.selector);
  for (const selector of ["ERS", "JERS-1", "RADARSAT-1"]) {
    assert.ok(!selectors.includes(selector), `${selector} is retired`);
    assert.ok(
      !fixture.datasets.some((item) => item.dataset === selector),
      `${selector} has no dataset capability`,
    );
  }
  assert.ok(selectors.includes("SEASAT"));
  assert.ok(selectors.includes("ALOS PALSAR"));
});

test("an executable historical ASF source with an incomplete projection fails visibly", () => {
  const legacy = source("ALOS PALSAR");
  const broken = {
    ...legacy,
    available: true,
    filters: { legacy_dataset: "ALOS PALSAR" },
  };
  const projected = datasetCapabilityForSarSource(broken);
  assert.equal(projected.ok, false);
  if (projected.ok) return;
  assert.match(projected.error, /ALOS PALSAR/);
  assert.match(projected.error, /capability|能力|投影/i);
});


test("NISAR band lookup switches complete source authority instead of writing a provider filter", () => {
  const lBand = source("NISAR");
  const sBand: SarSourceCapability = {
    ...lBand,
    selector: "NISAR-S",
    display_name: "NISAR S-band RSLC",
    logical_dataset: "sar.nisar.s.rslc.provisional.v1",
    provider_id: "bhoonidhi.nrsc",
    native_collection_id: "NISAR_S_RSLC",
    collection_profile_id: "bhoonidhi.nisar.s.rslc.v1",
    available: false,
    disabled_reason: "production_enabled=false",
    filters: {
      ...lBand.filters,
      fixed: { ...(lBand.filters.fixed as Record<string, unknown>), band: "S" },
    },
  };

  assert.equal(nisarSourceForBand([lBand, sBand], "L")?.provider_id, "asf.nasa.earthdata");
  assert.equal(nisarSourceForBand([lBand, sBand], "S")?.provider_id, "bhoonidhi.nrsc");
  assert.equal(nisarSourceForBand([lBand], "S"), null);
});

test("NISAR search admission rejects a missing, unavailable, or mismatched band source", () => {
  const lBand = { ...source("NISAR"), available: true, disabled_reason: null };
  const sBand: SarSourceCapability = {
    ...lBand,
    selector: "NISAR-S",
    display_name: "NISAR S-band RSLC",
    logical_dataset: "sar.nisar.s.rslc.provisional.v1",
    provider_id: "bhoonidhi.nrsc",
    native_collection_id: "NISAR_S_RSLC",
    collection_profile_id: "bhoonidhi.nisar.s.rslc.v1",
    available: false,
    disabled_reason: "production_enabled=false",
    filters: {
      ...lBand.filters,
      fixed: { ...(lBand.filters.fixed as Record<string, unknown>), band: "S" },
    },
  };

  const missing = nisarSearchSourceAdmission([lBand], "S", lBand);
  const unavailable = nisarSearchSourceAdmission([lBand, sBand], "S", sBand);
  const mismatched = nisarSearchSourceAdmission([lBand, { ...sBand, available: true }], "S", lBand);
  const admitted = nisarSearchSourceAdmission([lBand], "L", lBand);

  assert.equal(missing.ok, false);
  assert.equal(unavailable.ok, false);
  assert.equal(mismatched.ok, false);
  assert.equal(admitted.ok, true);
  if (!missing.ok) assert.match(missing.error, /S-band.*正在接入/);
  if (!unavailable.ok) assert.match(unavailable.error, /S-band.*暂不可检索/);
  if (!mismatched.ok) assert.match(mismatched.error, /波段与数据源不一致/);
  if (admitted.ok) assert.equal(admitted.source.provider_id, "asf.nasa.earthdata");
});

test("NISAR subset capability reasons are translated into actionable user copy", () => {
  const status = (state: string, reason: string) => ({
    resource_id: "nisar",
    capability: { capability_id: "subset", kind: "subset", lifecycle: "active" },
    inputs: {
      build: null,
      runtime: null,
      credential: null,
      admission: null,
      policy: null,
    },
    projection: { state, reason },
  });

  assert.match(
    nisarSubsetCapabilityMessage(status("blocked_credential", "credential_missing_or_unknown") as never, "L"),
    /Earthdata.*账号/,
  );
  assert.match(
    nisarSubsetCapabilityMessage(status("blocked_admission", "admission_missing_rejected_or_unknown") as never, "S"),
    /尚未完成验收/,
  );
  assert.doesNotMatch(
    nisarSubsetCapabilityMessage(status("blocked_runtime", "runtime_not_ready_or_unknown") as never, "L"),
    /[_a-z]{3,}/,
  );
});

test("admitted Bhoonidhi S-band uses the exact collection route without forcing one polarization", () => {
  const lBand = source("NISAR");
  const sBand: SarSourceCapability = {
    selector: "NISAR_SSAR",
    display_name: "NISAR S-band SSAR RSLC",
    search_kind: "typed_catalog_profile",
    logical_dataset: "sar.nisar.s.ssar.rslc.v1",
    provider_id: "isro.nrsc.bhoonidhi",
    native_collection_id: "NISAR_SSAR_RSLC",
    collection_profile_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1",
    available: true,
    disabled_reason: null,
    filters: {
      supported_keys: [
        "datetime", "online", "mode", "polarization", "acquisition_mode",
        "product_code", "software_version", "producer", "platform", "instrument",
        "scene_identifier",
      ],
      fixed: {
        platform: "NISAR",
        instrument: "SSAR",
        band: "S",
        product_type: "RSLC",
        processing_level: "L1",
        online: "Y",
      },
      options: {
        mode: ["W"],
        polarization: ["SH", "SV", "DH", "DV", "CP"],
        acquisition_mode: ["J", "W"],
        software_version: ["1.4.00"],
      },
    },
    supported_actions: ["search"],
  };

  assert.equal(nisarSourceForBand([lBand, sBand], "S"), sBand);
  const admission = nisarSearchSourceAdmission([lBand, sBand], "S", sBand);
  assert.equal(admission.ok, true);
  assert.deepEqual(sarSourceOptionValues(sBand, "frequency_a_polarization"), []);
  assert.deepEqual(sarSourceOptionValues(sBand, "frequency_b_polarization"), []);
  assert.deepEqual(
    sarSourceOptionValues(sBand, "polarization"),
    ["SH", "SV", "DH", "DV", "CP"],
  );
  const projection = datasetCapabilityForSarSource(sBand);
  assert.equal(projection.ok, true);
  if (projection.ok) {
    assert.deepEqual(projection.capability.polarizations, ["SH", "SV", "DH", "DV", "CP"]);
  }

  const result = buildTypedSarCatalogQuery({
    source: sBand,
    aoiGeojson: aoi,
    startUtc: "2026-08-30",
    endUtc: "2026-08-30",
    resultLimit: 12,
    nisarFrameCoverage: "",
    nisarFrequencyA: "",
    nisarFrequencyB: "",
    nisarRangeBandwidth: "",
    nisarFlightDirection: "",
    nisarLookDirection: "",
    nisarProductType: "RSLC",
    nisarBand: "S",
    nisarMaturity: "",
    bhoonidhiMode: "",
    bhoonidhiPolarization: "DV",
    bhoonidhiAcquisitionMode: "J",
    bhoonidhiSoftwareVersion: "",
  });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.query.provider_id, "isro.nrsc.bhoonidhi");
  assert.equal(result.query.logical_dataset, "sar.nisar.s.ssar.rslc.v1");
  assert.equal(result.query.native_collection_id, "NISAR_SSAR_RSLC");
  assert.equal(result.query.filters.mode, "W");
  assert.equal(result.query.filters.polarization, "DV");
  assert.equal(result.query.filters.software_version, "1.4.00");
  assert.equal(result.query.filters.acquisition_mode, "J");
  for (const key of [
    "production_configuration",
    "band",
    "frequency_a_polarization",
    "frequency_b_polarization",
    "quality_status",
    "frame_coverage",
    "range_bandwidth",
    "flight_direction",
    "look_direction",
  ]) {
    assert.equal(result.query.filters[key], undefined, key);
  }

  const invalidSsarFilter = buildTypedSarCatalogQuery({
    source: sBand,
    aoiGeojson: null,
    startUtc: "",
    endUtc: "",
    resultLimit: null,
    nisarProductType: "RSLC",
    nisarBand: "S",
    bhoonidhiPolarization: "XX",
  });
  assert.equal(invalidSsarFilter.ok, false);
  if (!invalidSsarFilter.ok) assert.match(invalidSsarFilter.error, /polarization.*合法选项/);

  const rejected = buildTypedSarCatalogQuery({
    source: sBand,
    aoiGeojson: null,
    startUtc: "",
    endUtc: "",
    resultLimit: null,
    nisarFrequencyA: "RH",
    nisarProductType: "RSLC",
    nisarBand: "S",
  });
  assert.equal(rejected.ok, false);
  if (!rejected.ok) assert.match(rejected.error, /不支持 L-band frequency_a_polarization/);
});
