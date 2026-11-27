import assert from "node:assert/strict";
import test from "node:test";

import type {
  DesktopNisarAppendContextV2,
  DesktopNisarAppendPreflightV2Request,
  DesktopNisarSuccessorTaskV2Request,
} from "./desktopShellBridge.ts";
import {
  createTauriDesktopShellBridge,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";

const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

function coreRestoreResponse() {
  return {
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 4,
    },
    core: {},
  };
}

function dependenciesFor(
  invoke: TauriBridgeDependencies["invoke"],
): TauriBridgeDependencies {
  return {
    invoke,
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
}

function bridgeFor(invoke: TauriBridgeDependencies["invoke"]) {
  return createTauriDesktopShellBridge(
    dependenciesFor(invoke),
    "tauri://localhost/",
    false,
  );
}

function appendContext(
  overrides: Partial<DesktopNisarAppendContextV2> = {},
) {
  return {
    task_id: "source-task",
    source_batch_id: "source-batch",
    task_revision: 7,
    task_status: "queued" as const,
    frozen_aoi_geojson: geometry,
    selected_frequency: "A" as const,
    selected_polarizations: ["HH", "HV"],
    output_root: "E:/nisar-output",
    layout: "direct" as const,
    concurrency: 2,
    ...overrides,
  };
}

function eligibleItem(sourceItemId: string) {
  return {
    source_item_id: sourceItemId,
    effective_geometry: geometry,
    available_selections: [{ frequency: "A" as const, polarizations: ["HH", "HV"] }],
  };
}

test("NISAR append context accepts the exact typed response and named command", async () => {
  const request = { task_id: "source-task" };
  const response = appendContext();
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = bridgeFor(async (command, args) => {
    calls.push([command, args]);
    return response;
  });

  const result = await bridge.application.readNisarSubsetAppendContextV2(request);

  assert.deepEqual(result, { ok: true, context: response });
  assert.deepEqual(calls, [[
    "application_read_nisar_subset_append_context_v2",
    { request },
  ]]);
});

test("NISAR append context rejects identity mismatches and unsafe extra fields", async () => {
  const request = { task_id: "source-task" };
  const valid = appendContext();
  const malformed = [
    { ...valid, task_id: "different-task" },
    {
      ...valid,
      frozen_aoi_geojson: { ...valid.frozen_aoi_geojson, href: "https://untrusted.example/aoi" },
    },
    { ...valid, authorization: "must-not-cross-the-typed-response-boundary" },
  ];

  for (const response of malformed) {
    let callCount = 0;
    const bridge = bridgeFor(async (command, args) => {
      callCount += 1;
      assert.equal(command, "application_read_nisar_subset_append_context_v2");
      assert.deepEqual(args, { request });
      return response;
    });

    const result = await bridge.application.readNisarSubsetAppendContextV2(request);

    assert.equal(result.ok, false);
    assert.equal(callCount, 1);
  }
});

test("NISAR append preflight requires an exact eligible/excluded ID partition", async () => {
  const request: DesktopNisarAppendPreflightV2Request = {
    task_id: "source-task",
    expected_task_revision: 7,
    source_item_ids: ["scene-1", "scene-2"],
  };
  const valid = {
    task_id: request.task_id,
    source_batch_id: "source-batch",
    task_revision: request.expected_task_revision,
    eligible_items: [eligibleItem("scene-1"), eligibleItem("scene-2")],
    excluded_items: [],
    common_selections: [{ frequency: "A" as const, polarizations: ["HH", "HV"] }],
  };
  const malformed = [
    { ...valid, eligible_items: [eligibleItem("scene-1"), eligibleItem("scene-1")] },
    { ...valid, eligible_items: [eligibleItem("scene-1")], excluded_items: [] },
    { ...valid, eligible_items: [eligibleItem("scene-1"), eligibleItem("foreign-scene")] },
  ];

  for (const response of malformed) {
    let callCount = 0;
    const bridge = bridgeFor(async (command, args) => {
      callCount += 1;
      assert.equal(command, "application_preflight_nisar_subset_append_v2");
      assert.deepEqual(args, { request });
      return response;
    });

    const result = await bridge.application.preflightNisarSubsetAppendV2(request);

    assert.equal(result.ok, false);
    assert.equal(callCount, 1);
  }
});

test("NISAR append preflight accepts zero eligible items with typed frequency and polarization exclusions", async () => {
  const request: DesktopNisarAppendPreflightV2Request = {
    task_id: "source-task",
    expected_task_revision: 7,
    source_item_ids: ["scene-frequency", "scene-polarization"],
  };
  const response = {
    task_id: request.task_id,
    source_batch_id: "source-batch",
    task_revision: request.expected_task_revision,
    eligible_items: [],
    excluded_items: [
      { source_item_id: "scene-frequency", reason: "frequency_incompatible" as const },
      { source_item_id: "scene-polarization", reason: "polarization_incompatible" as const },
    ],
    common_selections: [],
  };
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = bridgeFor(async (command, args) => {
    calls.push([command, args]);
    return response;
  });

  const result = await bridge.application.preflightNisarSubsetAppendV2(request);

  assert.deepEqual(result, { ok: true, preflight: response });
  assert.deepEqual(calls, [[
    "application_preflight_nisar_subset_append_v2",
    { request },
  ]]);
});

test("NISAR successor rejects reusing the source task ID and forwards the exact named command", async () => {
  const request: DesktopNisarSuccessorTaskV2Request = {
    source_task_id: "source-task",
    expected_task_revision: 7,
    source_item_ids: ["scene-1"],
    new_task_id: "successor-task",
    output_root: "E:/nisar-output",
    layout: "direct",
    concurrency: 2,
    created_at: "2026-09-05T00:00:00Z",
  };
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const bridge = bridgeFor(async (command, args) => {
    calls.push([command, args]);
    return coreRestoreResponse();
  });

  const accepted = await bridge.application.createNisarSubsetSuccessorTaskV2(request);
  const rejected = await bridge.application.createNisarSubsetSuccessorTaskV2({
    ...request,
    new_task_id: request.source_task_id,
  });

  assert.equal(accepted.ok, true);
  assert.equal(rejected.ok, false);
  assert.deepEqual(calls, [[
    "application_create_nisar_subset_successor_task_v2",
    { request },
  ]]);
});
