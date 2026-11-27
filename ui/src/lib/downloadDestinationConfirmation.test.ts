import assert from "node:assert/strict";
import test from "node:test";

import {
  beginDownloadDestinationConfirmation,
  createDownloadAcquisitionDraft,
  downloadDestinationConfirmationReducer,
  initialDownloadDestinationConfirmationState,
  newTaskDestinationDecision,
} from "./downloadDestinationConfirmation.ts";

function acquisitionDraft() {
  return createDownloadAcquisitionDraft({
    sourceBatchId: "search-batch-7",
    sourceBatchRevision: 12,
    selectionScope: "checked",
    selectedProductIds: ["S1-A", "S1-B"],
    selectedProductCount: 2,
    acquisition: {
      kind: "sentinel1",
      frozenParameters: { asset_policy: "all" },
    },
    destinationPrefill: "F:/downloads/default",
    layout: "sentinel1_slc",
    taskPolicy: "manual",
    concurrency: 3,
  });
}

const LANDSAT_L2SP_KEYS = [
  "coastal", "blue", "green", "red", "nir08", "swir16", "swir22",
  "lwir11", "drad", "emis", "emsd", "trad", "urad", "atran", "cdist", "qa",
  "qa_pixel", "qa_radsat", "qa_aerosol", "ang", "mtl.txt", "mtl.xml",
] as const;

function landsatDraft() {
  const assets = LANDSAT_L2SP_KEYS.map((assetKey, index) => ({
    asset_key: assetKey,
    file_name: `LC09_TEST_${assetKey}.dat`,
    role: (["ang", "mtl.txt", "mtl.xml"].includes(assetKey)
      ? "metadata"
      : ["qa", "qa_pixel", "qa_radsat", "qa_aerosol"].includes(assetKey)
        ? "auxiliary"
        : "primary") as "primary" | "auxiliary" | "metadata",
    size_bytes: index + 1,
  }));
  return createDownloadAcquisitionDraft({
    sourceBatchId: "landsat-batch",
    sourceBatchRevision: 4,
    selectionScope: "exact_item",
    selectedProductIds: ["LANDSAT-A"],
    selectedProductCount: 1,
    acquisition: {
      kind: "landsat",
      frozenParameters: {
        delivery_form: "official_archive",
        official_archive_available: true,
        complete_asset_set_available: true,
        content_selection: "all",
        selected_asset_keys: assets.map((asset) => asset.asset_key),
        available_assets: assets,
        product_folder_name: "LC09_TEST_PRODUCT",
      },
    },
    destinationPrefill: "F:/downloads/landsat",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 4,
  });
}

function sentinel2Draft(safeArchiveAvailable = true) {
  return createDownloadAcquisitionDraft({
    sourceBatchId: "sentinel2-batch",
    sourceBatchRevision: 5,
    selectionScope: "exact_item",
    selectedProductIds: ["S2-A"],
    selectedProductCount: 1,
    acquisition: {
      kind: "sentinel2",
      frozenParameters: {
        delivery_form: "safe_archive",
        safe_archive_available: safeArchiveAvailable,
        analysis_bands_available: true,
        band_preset: "true_color",
        selected_band_keys: ["B02", "B03", "B04"],
      },
    },
    destinationPrefill: "F:/downloads/sentinel2",
    layout: "direct",
    taskPolicy: "manual",
    concurrency: 4,
  });
}

test("the global default only prefills a review and cannot confirm itself", () => {
  const draft = acquisitionDraft();
  const reviewing = beginDownloadDestinationConfirmation(draft);

  assert.equal(reviewing.status, "reviewing");
  assert.equal(reviewing.outputRoot, "F:/downloads/default");
  assert.equal(reviewing.confirmedDraft, null);
  assert.equal(Object.isFrozen(reviewing.draft), true);
  assert.equal(Object.isFrozen(reviewing.draft.selectedProductIds), true);
  assert.equal(Object.isFrozen(reviewing.draft.acquisition.frozenParameters), true);
});

test("cancel returns to idle without producing a confirmed Store mutation draft", () => {
  const reviewing = beginDownloadDestinationConfirmation(acquisitionDraft());
  const cancelled = downloadDestinationConfirmationReducer(reviewing, { type: "cancel" });

  assert.deepEqual(cancelled, initialDownloadDestinationConfirmationState);
});

test("confirmation freezes the source revision, selection, destination and task policy", () => {
  const draft = acquisitionDraft();
  const reviewing = beginDownloadDestinationConfirmation(draft);
  const changed = downloadDestinationConfirmationReducer(reviewing, {
    type: "directory_selected",
    outputRoot: "F:/downloads/this-task",
  });
  const submitting = downloadDestinationConfirmationReducer(changed, { type: "confirm" });

  assert.equal(submitting.status, "submitting");
  assert.deepEqual(submitting.confirmedDraft, {
    sourceBatchId: "search-batch-7",
    sourceBatchRevision: 12,
    selectionScope: "checked",
    selectedProductIds: ["S1-A", "S1-B"],
    selectedProductCount: 2,
    acquisition: {
      kind: "sentinel1",
      frozenParameters: { asset_policy: "all" },
    },
    outputRoot: "F:/downloads/this-task",
    layout: "sentinel1_slc",
    taskPolicy: "manual",
    concurrency: 3,
  });
  assert.equal(Object.isFrozen(submitting.confirmedDraft), true);
});

test("a submission failure preserves the immutable acquisition facts for retry", () => {
  const reviewing = beginDownloadDestinationConfirmation(acquisitionDraft());
  const submitting = downloadDestinationConfirmationReducer(reviewing, { type: "confirm" });
  const failed = downloadDestinationConfirmationReducer(submitting, {
    type: "submission_failed",
    message: "Store unavailable",
  });

  assert.equal(failed.status, "reviewing");
  assert.equal(failed.error, "Store unavailable");
  assert.strictEqual(failed.draft, reviewing.draft);
  assert.equal(failed.outputRoot, "F:/downloads/default");
  assert.equal(failed.confirmedDraft, null);
});

test("Store-owned checked selection freezes its authoritative count without copying hidden IDs", () => {
  const draft = createDownloadAcquisitionDraft({
    sourceBatchId: "store-owned-batch",
    sourceBatchRevision: 19,
    selectionScope: "checked",
    selectedProductIds: [],
    selectedProductCount: 37,
    acquisition: { kind: "sentinel1", frozenParameters: {} },
    destinationPrefill: "F:/downloads/default",
    layout: "sentinel1_slc",
    taskPolicy: "manual",
    concurrency: 2,
  });

  assert.equal(draft.selectedProductCount, 37);
  assert.deepEqual(draft.selectedProductIds, []);
  const submitting = downloadDestinationConfirmationReducer(
    beginDownloadDestinationConfirmation(draft),
    { type: "confirm" },
  );
  assert.equal(submitting.status, "submitting");
  assert.equal(submitting.confirmedDraft?.selectedProductCount, 37);
  assert.deepEqual(submitting.confirmedDraft?.selectedProductIds, []);
});

test("NISAR subset cannot enter destination review before scientific parameters are frozen", () => {
  assert.throws(
    () => createDownloadAcquisitionDraft({
      sourceBatchId: "nisar-batch",
      sourceBatchRevision: 2,
      selectionScope: "exact_item",
      selectedProductIds: ["NISAR-A"],
      acquisition: {
        kind: "nisar_subset",
        frozenParameters: {
          frequency: "A",
          polarizations: ["HH", "HV"],
          transfer_mode: "remote_derived_subset",
        },
      },
      destinationPrefill: "F:/downloads/nisar",
      layout: "direct",
      taskPolicy: "manual",
      concurrency: 1,
    }),
    /subset AOI/i,
  );
});

test("Sentinel-2 analysis delivery freezes scientific presets without changing SAFE semantics", () => {
  const reviewing = beginDownloadDestinationConfirmation(sentinel2Draft());
  const analysis = downloadDestinationConfirmationReducer(reviewing, {
    type: "sentinel2_delivery_changed",
    deliveryForm: "analysis_bands",
  });
  assert.equal(analysis.status, "reviewing");
  if (analysis.status !== "reviewing") return;
  assert.equal(analysis.draft.acquisition.frozenParameters.delivery_form, "analysis_bands");
  assert.deepEqual(
    analysis.draft.acquisition.frozenParameters.selected_band_keys,
    ["B02", "B03", "B04"],
  );

  const vegetation = downloadDestinationConfirmationReducer(analysis, {
    type: "sentinel2_band_preset_changed",
    preset: "vegetation",
  });
  assert.equal(vegetation.status, "reviewing");
  if (vegetation.status !== "reviewing") return;
  assert.deepEqual(
    vegetation.draft.acquisition.frozenParameters.selected_band_keys,
    ["B04", "B08"],
  );
});

test("Sentinel-2 custom analysis delivery rejects an empty reviewed-band set", () => {
  let state = downloadDestinationConfirmationReducer(
    beginDownloadDestinationConfirmation(sentinel2Draft(false)),
    { type: "sentinel2_delivery_changed", deliveryForm: "analysis_bands" },
  );
  state = downloadDestinationConfirmationReducer(
    state,
    { type: "sentinel2_band_preset_changed", preset: "custom" },
  );
  for (const bandKey of ["B02", "B03", "B04"]) {
    state = downloadDestinationConfirmationReducer(state, {
      type: "sentinel2_band_toggled",
      bandKey,
    });
  }
  assert.equal(state.status, "reviewing");
  if (state.status !== "reviewing") return;
  assert.deepEqual(state.draft.acquisition.frozenParameters.selected_band_keys, []);
  const blocked = downloadDestinationConfirmationReducer(state, { type: "confirm" });
  assert.equal(blocked.status, "reviewing");
  assert.equal(
    blocked.status === "reviewing" ? blocked.error : null,
    "请至少选择一个 Sentinel-2 分析波段。",
  );
});

test("Landsat SR and ST presets freeze exact official file groups and force expanded delivery", () => {
  const reviewing = beginDownloadDestinationConfirmation(landsatDraft());
  const reflectance = downloadDestinationConfirmationReducer(reviewing, {
    type: "landsat_content_changed",
    selection: "surface_reflectance",
  });
  assert.equal(reflectance.status, "reviewing");
  if (reflectance.status !== "reviewing") return;
  const reflectanceParameters = reflectance.draft.acquisition.frozenParameters;
  assert.equal(reflectanceParameters.delivery_form, "complete_asset_set");
  assert.equal(reflectanceParameters.content_selection, "surface_reflectance");
  assert.equal((reflectanceParameters.selected_asset_keys as string[]).length, 13);
  assert.ok((reflectanceParameters.selected_asset_keys as string[]).includes("red"));
  assert.equal((reflectanceParameters.selected_asset_keys as string[]).includes("lwir11"), false);

  const temperature = downloadDestinationConfirmationReducer(reviewing, {
    type: "landsat_content_changed",
    selection: "surface_temperature",
  });
  assert.equal(temperature.status, "reviewing");
  if (temperature.status !== "reviewing") return;
  const temperatureParameters = temperature.draft.acquisition.frozenParameters;
  assert.equal(temperatureParameters.delivery_form, "complete_asset_set");
  assert.equal((temperatureParameters.selected_asset_keys as string[]).length, 14);
  assert.ok((temperatureParameters.selected_asset_keys as string[]).includes("lwir11"));
  assert.equal((temperatureParameters.selected_asset_keys as string[]).includes("coastal"), false);
});

test("Landsat custom selection can remove exact files but cannot confirm an empty file set", () => {
  let state = downloadDestinationConfirmationReducer(
    beginDownloadDestinationConfirmation(landsatDraft()),
    { type: "landsat_content_changed", selection: "custom" },
  );
  assert.equal(state.status, "reviewing");
  for (const assetKey of LANDSAT_L2SP_KEYS) {
    state = downloadDestinationConfirmationReducer(state, {
      type: "landsat_asset_toggled",
      assetKey,
    });
  }
  assert.equal(state.status, "reviewing");
  if (state.status !== "reviewing") return;
  assert.deepEqual(state.draft.acquisition.frozenParameters.selected_asset_keys, []);
  const blocked = downloadDestinationConfirmationReducer(state, { type: "confirm" });
  assert.equal(blocked.status, "reviewing");
  assert.equal(blocked.status === "reviewing" ? blocked.error : null, "请至少选择一个 Landsat 文件。");
});

test("Landsat partial content cannot switch back to archive delivery", () => {
  const reviewing = downloadDestinationConfirmationReducer(
    beginDownloadDestinationConfirmation(landsatDraft()),
    { type: "landsat_content_changed", selection: "surface_reflectance" },
  );
  const attemptedArchive = downloadDestinationConfirmationReducer(reviewing, {
    type: "landsat_delivery_changed",
    deliveryForm: "official_archive",
  });
  assert.strictEqual(attemptedArchive, reviewing);
  assert.equal(
    reviewing.status === "reviewing"
      ? reviewing.draft.acquisition.frozenParameters.delivery_form
      : null,
    "complete_asset_set",
  );
});

test("append reuses an actionable task destination while every successor is a confirmed new task", () => {
  for (const status of ["queued", "running", "paused"] as const) {
    assert.deepEqual(newTaskDestinationDecision({ intent: "append", taskStatus: status }), {
      kind: "reuse_existing_task",
    });
  }
  for (const status of ["completed", "cancelled", "failed"] as const) {
    assert.deepEqual(newTaskDestinationDecision({ intent: "create_successor", taskStatus: status }), {
      kind: "confirm_new_task",
    });
  }
  assert.deepEqual(newTaskDestinationDecision({ intent: "create_new", taskStatus: null }), {
    kind: "confirm_new_task",
  });
});
