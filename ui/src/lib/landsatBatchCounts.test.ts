import assert from "node:assert/strict";
import test from "node:test";
import { parseCoreRestore, parseCoreTaskSummary } from "./coreRestoreProjection.ts";

test("Landsat batch admission counts scenes while Store selected_count counts transfer files", () => {
  const payload = {
    task_id:"batch-task",batch_id:"landsat-assets-batch-batch-task",provider_id:"microsoft.planetary-computer",
    dataset_id:"optical.landsat.c2.l2",created_at:"2026-09-29T00:00:00Z",updated_at:"2026-09-29T00:00:00Z",
    status:"queued",revision:0,selected_count:22,checked_count:22,delivery_requested_count:2,delivery_rejected_count:1,
    active_count:0,paused_count:0,queued_count:22,completed_count:0,failed_count:0,skipped_count:0,cancelled_count:0,
    terminal_bytes:0,active_bytes:0,other_bytes:0,transferred_bytes:0,total_bytes:null,bytes_per_second:0,transfer_active_millis:0,
    active_item:null,output_root:"C:\\fixtures",layout:"direct",operation:null,actionable:true,
  };
  assert.ok(parseCoreTaskSummary(payload), "1 admitted scene is represented by 22 transfer files, not 22 admitted scenes");
  assert.ok(parseCoreTaskSummary({...payload,selected_count:44,checked_count:44,queued_count:44,delivery_rejected_count:0}));
  for (const invalid of [
    {...payload,delivery_rejected_count:2}, {...payload,delivery_rejected_count:3},
    {...payload,delivery_requested_count:251}, {...payload,selected_count:0},
    {...payload,batch_id:"unrelated-batch"}, {...payload,provider_id:"usgs.eros.m2m"},
    {...payload,dataset_id:"optical.sentinel-2.l2a"},
  ]) assert.equal(parseCoreTaskSummary(invalid),null);

  const rustPayloadWithCompanionFields = {
    ...payload,
    source_task_id: null,
    auxiliary_kind: null,
  };
  assert.ok(parseCoreTaskSummary(rustPayloadWithCompanionFields), "Rust StoreTaskSummary serializes source_task_id and auxiliary_kind");

  const singleSceneAssetPayload = {
    ...rustPayloadWithCompanionFields,
    batch_id: "landsat-assets-batch-task",
    provider_id: "element84.earth-search",
    selected_count: 11,
    checked_count: 11,
    queued_count: 11,
    delivery_requested_count: 1,
    delivery_rejected_count: 0,
  };
  assert.ok(parseCoreTaskSummary(singleSceneAssetPayload), "Single-scene or element84 fallback Landsat expanded asset task parses cleanly");

  const bundleBatchPayload = {
    ...rustPayloadWithCompanionFields,
    batch_id: "landsat-bundles-batch-batch-task",
    provider_id: "usgs.eros.m2m",
    selected_count: 1,
    checked_count: 1,
    queued_count: 1,
    delivery_requested_count: 2,
    delivery_rejected_count: 1,
  };
  assert.ok(parseCoreTaskSummary(bundleBatchPayload), "Landsat official archive batch task with companion fields parses cleanly");

  const coreWithOpticalTasks = {
    readiness: { schema_version: 11, ready: true },
    workspace_context: null,
    region_aoi: null,
    region_aois: [],
    dem_source_identity: null,
    tasks: [
      singleSceneAssetPayload,
      bundleBatchPayload,
    ],
    legacy_task_warnings: [],
    workbench: null,
  };
  const restored = parseCoreRestore(coreWithOpticalTasks);
  assert.ok(restored, "CoreRestore with single-scene expanded assets and official bundle batch tasks parses cleanly");
  assert.equal(restored.tasks.length, 2);
  assert.equal(
    parseCoreRestore({
      ...coreWithOpticalTasks,
      tasks: [...coreWithOpticalTasks.tasks, { ...bundleBatchPayload, task_id: "" }],
    }),
    null,
  );
});
