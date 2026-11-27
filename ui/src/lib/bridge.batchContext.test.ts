import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import type { CoreWorkbenchSummaryProjection } from "./coreRestoreProjection.ts";

let bridgeImportSequence = 0;

async function batchContextFromCoreWorkbench(summary: CoreWorkbenchSummaryProjection) {
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@/data/adminBoundaries") {
        return { url: new URL("../data/adminBoundaries.ts", import.meta.url).href, shortCircuit: true };
      }
      if (specifier === "./sarSearchForm") {
        return { url: new URL("./sarSearchForm.ts", context.parentURL).href, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });
  try {
    bridgeImportSequence += 1;
    const bridge = await import(`./bridge.ts?batch-context-${bridgeImportSequence}`);
    return bridge.batchContextFromCoreWorkbench(summary);
  } finally {
    hooks.deregister();
  }
}

function summary(dataset: string): CoreWorkbenchSummaryProjection {
  return {
    batch_id: "batch-1",
    provider_id: "asf.nasa.earthdata",
    created_at: "2026-08-01T00:00:00Z",
    aoi_name: "AOI",
    query: {
      provider: "asf.nasa.earthdata",
      dataset,
      start: null,
      end: null,
      target_count: 1,
    },
    search_stats: null,
    candidate_count: 1,
    checked_count: 1,
    current_page_item_count: 1,
    page_index: 0,
    page_size: 50,
    revision: 1,
    deferred: false,
    reason_code: null,
  };
}

test("core NISAR workbench restores the logical dataset identity", async () => {
  const context = await batchContextFromCoreWorkbench(
    summary("sar.nisar.l.rslc.provisional.v1"),
  );

  assert.equal(context.query.logical_dataset, "sar.nisar.l.rslc.provisional.v1");
});

test("core non-NISAR workbench does not synthesize a logical dataset identity", async () => {
  const context = await batchContextFromCoreWorkbench(summary("sar.sentinel-1.slc"));

  assert.equal(Object.hasOwn(context.query, "logical_dataset"), false);
});

test("core workbench carries bounded read-only provider fallback provenance", async () => {
  const traced = summary("optical.landsat.c2-l2");
  traced.provider_id = "element84.earth-search";
  traced.search_stats = {
    matched_count: null,
    provider_returned_count: 1,
    frozen_candidate_count: 1,
    truncated: false,
    source: "TYPED_CATALOG",
    source_attempts: [
      {
        provider_id: "microsoft.planetary-computer",
        native_collection_id: "landsat-c2-l2",
        outcome: "fallback",
        fallback_reason: "transient_network",
      },
      {
        provider_id: "element84.earth-search",
        native_collection_id: "landsat-c2-l2",
        outcome: "succeeded",
        fallback_reason: null,
      },
    ],
  };

  const context = await batchContextFromCoreWorkbench(traced);
  assert.deepEqual(context.source_attempts, traced.search_stats.source_attempts);
});
