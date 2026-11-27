import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  sarTaskDatasetLabel,
  sarTaskDisplayName,
  sarTaskTypeLabel,
  taskDeliveryProvider,
} from "./sarTaskDisplayName.ts";

test("task names use the reviewed ASF dataset display name", () => {
  const contract = JSON.parse(
    readFileSync(
      new URL("../../../contracts/v3/asf-capabilities.v1.json", import.meta.url),
      "utf8",
    ),
  ) as { datasets: { dataset: string; display_name: string }[] };

  for (const { dataset, display_name: displayName } of contract.datasets) {
    assert.equal(
      sarTaskDisplayName({
        snapshot_scenes: [{ sar_product: { dataset } }],
      }),
      `${displayName} 下载任务`,
    );
  }
});

test("active cards and restored history use the same frozen dataset identity", () => {
  assert.equal(
    sarTaskDisplayName({
      snapshot_scenes: [{ sar_product: { dataset: "ALOS PALSAR" } }],
      name: "ASF SAR product download task",
    }),
    "ALOS PALSAR 下载任务",
  );
  assert.equal(
    sarTaskDisplayName({
      snapshot: [{ dataset: "ALOS PALSAR" }],
      name: "ASF SAR product download task",
    }),
    "ALOS PALSAR 下载任务",
  );
});

test("a compact restored task can recover its dataset from the frozen batch query", () => {
  const task = {
    name: "ASF SAR product download task",
    batch_context: { query: { dataset: "ERS" } },
  };

  // ERS was removed from ASF's searchable interfaces (Owner evidence
  // 2026-10-31); a historical frozen task still recovers its reviewed label,
  // now carrying the ASF-removed marker.
  assert.equal(sarTaskDatasetLabel(task), "ERS-1 / ERS-2（ASF 已下线）");
  assert.equal(sarTaskDisplayName(task), "ERS-1 / ERS-2（ASF 已下线） 下载任务");
});

test("provider implementation names never leak into the generic fallback", () => {
  assert.equal(
    sarTaskDisplayName({ name: "ASF SAR product download task" }),
    "SAR 下载任务",
  );
  assert.equal(sarTaskDisplayName({}), "SAR 下载任务");
  assert.equal(sarTaskDatasetLabel({}), "SAR");
});

test("legacy Sentinel-1 tasks remain recognizable without a dataset field", () => {
  assert.equal(
    sarTaskDisplayName({ name: "Sentinel-1 download task" }),
    "Sentinel-1 下载任务",
  );
});

test("an unreviewed dataset remains specific instead of becoming ASF SAR", () => {
  assert.equal(
    sarTaskDisplayName({ snapshot: [{ dataset: "CUSTOM-SAR" }] }),
    "CUSTOM-SAR 下载任务",
  );
});

test("remote subset history is explicitly client-derived", () => {
  const task = {
    batch_context: { query: { delivery_kind: "remote_derived_subset" } },
  };
  assert.equal(sarTaskDatasetLabel(task), "NISAR 派生子集 · client-derived");
  assert.equal(sarTaskDisplayName(task), "NISAR 远程派生子集（client-derived）");
});

test("companion Orbit task retains its scientific identity in the shared queue", () => {
  assert.equal(sarTaskDisplayName({ task_type: "orbit" }), "Sentinel-1 精密轨道下载");
});

test("GACOS task labels follow the approved worker operation", () => {
  assert.equal(sarTaskDisplayName({ approved_worker_operation: "gacos_manual_request" }), "GACOS 请求");
  assert.equal(sarTaskDisplayName({ approved_worker_operation: "gacos_import_result" }), "GACOS 本地结果导入");
  assert.equal(sarTaskDisplayName({ approved_worker_operation: "gacos_fetch_result" }), "GACOS 结果下载");
  assert.equal(sarTaskDatasetLabel({ approved_worker_operation: "gacos_fetch_result" }), "GACOS");
});

test("ordinary download task types use their reviewed dataset instead of an empty dash", () => {
  assert.equal(sarTaskTypeLabel({ dataset_id: "SENTINEL-1" }), "Sentinel-1 SLC");
  assert.equal(sarTaskTypeLabel({ dataset_id: "optical.sentinel-2.l2a" }), "Sentinel-2 L2A");
  assert.equal(sarTaskTypeLabel({ dataset_id: "optical.landsat.c2.l2" }), "Landsat C2 L2");
});

test("Sentinel-2 analysis-band tasks remain distinct from complete SAFE delivery", () => {
  const task = {
    batch_id: "sentinel2-bands-task-1",
    dataset_id: "optical.sentinel-2.l2a",
  };
  assert.equal(sarTaskTypeLabel(task), "Sentinel-2 L2A · 分析波段");
  assert.equal(sarTaskDisplayName(task), "Sentinel-2 L2A 分析波段下载任务");
});

test("delivery provider follows the actual derived delivery route instead of the search source", () => {
  assert.deepEqual(taskDeliveryProvider({
    batch_id: "landsat-assets-task-1",
    provider_id: "usgs.eros.m2m",
  }), {
    id: "microsoft.planetary-computer",
    label: "MPC",
    officialUrl: "https://planetarycomputer.microsoft.com/",
  });
  assert.equal(taskDeliveryProvider({
    batch_id: "landsat-delivery-task-1",
    provider_id: "microsoft.planetary-computer",
  }).label, "USGS");
  assert.equal(taskDeliveryProvider({
    batch_id: "sentinel2-archive-task-1",
    provider_id: "microsoft.planetary-computer",
  }).label, "CDSE");
  assert.equal(taskDeliveryProvider({
    batch_id: "sentinel2-bands-task-1",
    provider_id: "copernicus.dataspace.odata",
  }).label, "MPC");
  assert.equal(taskDeliveryProvider({ provider_id: "asf.sentinel1" }).label, "ASF");
  assert.equal(taskDeliveryProvider({ provider_id: "asf.catalog" }).label, "ASF");
  assert.equal(taskDeliveryProvider({ provider_id: "isro.nrsc.bhoonidhi" }).label, "Bhoonidhi");
  assert.equal(taskDeliveryProvider({
    snapshot: [{
      provider: "isro.nrsc.bhoonidhi",
      sar_product: { dataset: "sar.nisar.s.ssar.rslc.v1", provider: "isro.nrsc.bhoonidhi" },
    }],
  }).label, "Bhoonidhi");
  assert.equal(sarTaskTypeLabel({
    snapshot: [{
      provider: "isro.nrsc.bhoonidhi",
      sar_product: { dataset: "sar.nisar.s.ssar.rslc.v1", provider: "isro.nrsc.bhoonidhi" },
    }],
  }), "NISAR S-band SSAR RSLC");
  assert.equal(taskDeliveryProvider({
    provider_id: "dem",
    operation: { source_provider: "opentopography.dem" },
  }).label, "OTG");
});
