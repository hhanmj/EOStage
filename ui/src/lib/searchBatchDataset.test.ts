import assert from "node:assert/strict";
import test from "node:test";

import type { SarSourceCapability } from "./generated/v3-contracts.ts";
import {
  isNisarSearchBatchQuery,
  resolveRestoredSarSource,
  restoredSearchFormSeed,
} from "./searchBatchDataset.ts";

test("NISAR acquisition identity comes from the immutable typed SearchBatch query", () => {
  assert.equal(isNisarSearchBatchQuery({
    logical_dataset: "sar.nisar.l.rslc.provisional.v1",
    filters: {},
  }), true);
  assert.equal(isNisarSearchBatchQuery({ logical_dataset: "sar.alos-2.palsar-2.scansar.l1.1.v1" }), false);
  assert.equal(isNisarSearchBatchQuery({ dataset: "NISAR" }), false);
  assert.equal(isNisarSearchBatchQuery(null), false);
});

test("a restored batch query seeds the search form with its exact dataset, calendar dates and target", () => {
  assert.deepEqual(
    restoredSearchFormSeed({ provider: "ASF", dataset: "SLC-BURST", start: "2026-10-01", end: "2026-10-19", max_results: 120 }),
    { dataset: "SLC-BURST", start: "2026-10-01", end: "2026-10-19", datesInvalid: false, targetCount: 120 },
  );
  // Typed catalogue batches name the logical dataset; an instant keeps its calendar day.
  assert.deepEqual(
    restoredSearchFormSeed({ logical_dataset: "sar.nisar.l.rslc.provisional.v1", start: "2026-10-01T00:00:00Z", end: null, result_limit: 30 }),
    { dataset: "sar.nisar.l.rslc.provisional.v1", start: "2026-10-01", end: "", datesInvalid: false, targetCount: 30 },
  );
  assert.deepEqual(
    restoredSearchFormSeed({ dataset: "optical.hls.s30.v2", start: null, end: null, max_results: null }),
    { dataset: "optical.hls.s30.v2", start: "", end: "", datesInvalid: false, targetCount: null },
  );
});

test("a restored date the form cannot show is flagged instead of half-applied", () => {
  assert.deepEqual(
    restoredSearchFormSeed({ dataset: "SENTINEL-1", start: "20261001", end: "2026-10-19", max_results: 20 }),
    { dataset: "SENTINEL-1", start: "", end: "", datesInvalid: true, targetCount: 20 },
  );
  assert.equal(restoredSearchFormSeed({ dataset: "SENTINEL-1", start: 20261001, end: null })?.datesInvalid, true);
});

test("a restored query without a dataset gives no seed", () => {
  assert.equal(restoredSearchFormSeed({ provider: "ASF", dataset: null, start: "2026-10-01" }), null);
  assert.equal(restoredSearchFormSeed({ dataset: "" }), null);
  assert.equal(restoredSearchFormSeed(null), null);
  assert.equal(restoredSearchFormSeed(["SLC-BURST"]), null);
});

test("a restored dataset resolves to a source by selector, then by logical dataset", () => {
  const source = (selector: string, logicalDataset: string | null) => ({
    selector,
    logical_dataset: logicalDataset,
  }) as SarSourceCapability;
  const sources = [
    source("SENTINEL-1", null),
    source("SLC-BURST", null),
    source("NISAR", "sar.nisar.l.rslc.provisional.v1"),
  ];
  assert.equal(resolveRestoredSarSource("SLC-BURST", sources), sources[1]);
  assert.equal(resolveRestoredSarSource("sar.nisar.l.rslc.provisional.v1", sources), sources[2]);
  assert.equal(resolveRestoredSarSource("SAR-RETIRED", sources), null);
  assert.equal(resolveRestoredSarSource("slc-burst", sources), null);
});
