import assert from "node:assert/strict";
import test from "node:test";

import { projectTaskPagePresentation, taskFailureStage } from "./taskPagePresentation.ts";

test("task failure stage distinguishes writer, output, and unknown failures", () => {
  assert.equal(taskFailureStage("NISAR_SUBSET_WRITER_FAILED"), "生成 HDF5");
  assert.equal(taskFailureStage("NISAR_SUBSET_ARTIFACT_FAILED"), "生成 HDF5");
  assert.equal(taskFailureStage("NISAR_SUBSET_OUTPUT_COMMIT_FAILED"), "输出");
  assert.equal(taskFailureStage("NISAR_SUBSET_FUTURE_FAILURE"), "未知阶段");
});

test("a refused one-time budget upgrade is shown as local validation, not a range download", () => {
  assert.equal(taskFailureStage("NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED"), "本地校验");
  assert.equal(taskFailureStage("nisar_subset_v2_budget_upgrade_refused"), "本地校验");
  // A real transfer budget exhaustion keeps its existing download stage.
  assert.equal(taskFailureStage("NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED"), "下载数据范围");
});

test("login and session failures are shown as their own stage", () => {
  for (const code of [
    "DL004",
    "dl004",
    "CDSE_OAUTH_SESSION_UNAVAILABLE",
    "BHOONIDHI_CREDENTIALS_MISSING",
    "LANDSAT_P4A3_LOGIN_FAILED",
    "CDSE_ODATA_HTTP_UNAUTHORIZED",
    "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED",
  ]) {
    assert.equal(taskFailureStage(code), "登录/会话", code);
  }
  // Scientific authority codes are not logins.
  assert.equal(taskFailureStage("NISAR_SUBSET_V2_AUTHORITY_MISMATCH"), "未知阶段");
  assert.equal(taskFailureStage("NISAR_SUBSET_V2_SCIENCE_AUTHORITY_INVALID"), "校验");
});

test("task page presentation retains bytes, unknown totals, destination and retry truth", () => {
  const page = projectTaskPagePresentation({
    task_id: "task-1",
    revision: 7,
    page_index: 2,
    page_size: 100,
    total_count: 250,
    network_bytes_received: null,
    source_total_bytes: null,
    items: [
      {
        ordinal: 200,
        checked: true,
        item: {
          item_id: "scene-unknown",
          provider_id: "asf.sentinel1",
          display_name: "S1_UNKNOWN",
          acquisition_datetime: null,
          footprint_geojson: null,
          metadata: {
            delivery_kind: "remote_derived_subset_v2",
            nisar_subset_prepared_intent_v2_envelope: {
              source_batch_id: "nisar-batch",
              source_item_id: "scene-unknown",
              workbench_revision: 7,
              intent: { selected_frequency: "A", selected_polarizations: ["HH"] },
            },
            nisar_subset_plan_v2_envelope: { plan_contract: "nisar_subset_plan_v2" },
          },
          payload: {},
          asset_access: null,
          execution_policy: null,
        },
        status: "interrupted",
        bytes_completed: 55,
        total_bytes: null,
        source_total_bytes: null,
        bytes_per_second: 12,
        attempt: 2,
        destination: "C:\\downloads\\scene.zip.part",
        failure: { code: "NETWORK", reason_zh: "网络中断", retryable: true },
        finished_at: "2026-08-17T00:00:00Z",
        retryable: true,
      },
    ],
  });

  assert.equal(page.totalCount, 250);
  assert.equal(page.items[0]?.bytesCompleted, 55);
  assert.equal(page.items[0]?.totalBytes, null);
  assert.equal(page.items[0]?.percentage, null);
  assert.equal(page.items[0]?.bytesPerSecond, 0);
  assert.equal(page.items[0]?.destination, "C:\\downloads\\scene.zip.part");
  assert.equal(page.items[0]?.failure?.retryable, true);
  assert.deepEqual(page.items[0]?.metadata, {
    delivery_kind: "remote_derived_subset_v2",
    nisar_subset_prepared_intent_v2_envelope: {
      source_batch_id: "nisar-batch",
      source_item_id: "scene-unknown",
      workbench_revision: 7,
      intent: { selected_frequency: "A", selected_polarizations: ["HH"] },
    },
    nisar_subset_plan_v2_envelope: { plan_contract: "nisar_subset_plan_v2" },
  });
});
