import assert from "node:assert/strict";
import test from "node:test";

import type {
  DesktopApplicationBridge,
  DesktopCoreRestoreSnapshot,
} from "./desktopShellBridge.ts";
import { ReadonlyApplicationServiceCoordinator } from "./applicationServiceCoordinator.ts";

function coreSnapshot(
  workbenchRevision: number | null,
  taskRevision: number | null,
  taskId = "task-1",
): DesktopCoreRestoreSnapshot {
  return {
    ok: true,
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 4,
    },
    core: {
      readiness: { schema_version: 4, ready: true },
      workspace_context: null,
      region_aoi: null,
      region_aois: [],
      tasks: taskRevision === null
        ? []
        : [{
            task_id: taskId,
            batch_id: "batch-1",
            provider_id: "provider-1",
            created_at: "2026-08-13T00:00:00Z",
            status: "queued",
            revision: taskRevision,
            updated_at: "2026-08-13T00:00:00Z",
            selected_count: 0,
            checked_count: 0,
            active_count: 0,
            paused_count: 0,
            queued_count: 0,
            completed_count: 0,
            failed_count: 0,
            skipped_count: 0,
            cancelled_count: 0,
            terminal_bytes: 0,
            active_bytes: 0,
            other_bytes: 0,
            transferred_bytes: 0,
            total_bytes: null,
            bytes_per_second: 0,
      transfer_active_millis: 0,
            active_item: null,
            output_root: "D:/fixture",
            layout: "direct",
            operation: null,
            actionable: true,
          }],
      legacy_task_warnings: [],
      workbench: workbenchRevision === null
        ? null
        : {
            batch_id: "batch-1",
            provider_id: "provider-1",
            created_at: "2026-08-13T00:00:00Z",
            aoi_name: "AOI",
            query: {
              provider: "provider-1",
              dataset: "DATASET",
              start: null,
              end: null,
              target_count: null,
            },
            search_stats: null,
            candidate_count: 10_000,
            checked_count: 0,
            current_page_item_count: 100,
            page_index: 0,
            page_size: 100,
            revision: workbenchRevision,
            deferred: true,
            reason_code: "candidate_payload_deferred",
          },
    },
  };
}

function bridgeFrom(reads: Array<Promise<DesktopCoreRestoreSnapshot>>): DesktopApplicationBridge {
  return {
    coreRestore: async () => reads.shift()!,
  } as DesktopApplicationBridge;
}

test("coordinator publishes CoreRestore revisions once and rejects duplicate or stale revisions", async () => {
  const values: string[] = [];
  const first = Promise.resolve(coreSnapshot(2, 4));
  const duplicate = Promise.resolve(coreSnapshot(2, 4));
  const newer = Promise.resolve(coreSnapshot(2, 5));
  const coordinator = new ReadonlyApplicationServiceCoordinator({
    bridge: bridgeFrom([first, duplicate, newer]),
    publish: (value) => {
      if (value.ok) values.push(`${value.readinessStatus}:${value.revisionCount}`);
    },
  });

  assert.ok(await coordinator.poll());
  assert.equal(await coordinator.refresh(), null);
  assert.ok(await coordinator.refresh());
  assert.deepEqual(values, ["store_ready:2", "store_ready:2"]);
  coordinator.dispose();
});

test("reconnect drops an old CoreRestore response and accepts the next snapshot at the same revision", async () => {
  let resolveOld!: (value: DesktopCoreRestoreSnapshot) => void;
  let resolveFresh!: (value: DesktopCoreRestoreSnapshot) => void;
  const old = new Promise<DesktopCoreRestoreSnapshot>((resolve) => {
    resolveOld = resolve;
  });
  const fresh = new Promise<DesktopCoreRestoreSnapshot>((resolve) => {
    resolveFresh = resolve;
  });
  const values: number[] = [];
  const coordinator = new ReadonlyApplicationServiceCoordinator({
    bridge: bridgeFrom([old, fresh]),
    publish: (value) => {
      if (value.ok) values.push(value.revisionCount);
    },
  });

  const oldRequest = coordinator.poll();
  assert.ok(oldRequest);
  await Promise.resolve();
  coordinator.reconnect();
  const freshRequest = coordinator.poll();
  assert.ok(freshRequest);

  resolveOld(coreSnapshot(7, 8));
  assert.equal(await oldRequest, null);
  resolveFresh(coreSnapshot(7, 8));
  assert.ok(await freshRequest);
  assert.deepEqual(values, [2]);
  coordinator.dispose();
});

test("invalid CoreRestore shape is projected as a bounded typed error", async () => {
  const bridge: DesktopApplicationBridge = {
    coreRestore: async () => ({
      ok: true,
      readiness: {
        contract: "application_service_readonly_v1",
        status: "store_ready",
        store_schema_version: 4,
      },
      core: {},
    }),
  } as DesktopApplicationBridge;
  let value;
  const coordinator = new ReadonlyApplicationServiceCoordinator({
    bridge,
    publish: (status) => {
      value = status;
    },
  });

  await coordinator.poll();
  assert.equal(value?.ok, false);
  assert.equal(value?.code, "V3_APPLICATION_INVALID_SNAPSHOT");
  coordinator.dispose();
});

test("empty CoreRestore remains publishable while invalid task IDs are rejected", async () => {
  const emptyBridge: DesktopApplicationBridge = {
    coreRestore: async () => coreSnapshot(null, null),
  } as DesktopApplicationBridge;
  let emptyPublishes = 0;
  const emptyCoordinator = new ReadonlyApplicationServiceCoordinator({
    bridge: emptyBridge,
    publish: () => {
      emptyPublishes += 1;
    },
  });
  await emptyCoordinator.poll();
  await emptyCoordinator.refresh();
  assert.equal(emptyPublishes, 2);
  emptyCoordinator.dispose();

  const invalidIdBridge: DesktopApplicationBridge = {
    coreRestore: async () => coreSnapshot(null, 0, ""),
  } as DesktopApplicationBridge;
  let invalidIdStatus;
  const invalidIdCoordinator = new ReadonlyApplicationServiceCoordinator({
    bridge: invalidIdBridge,
    publish: (status) => {
      invalidIdStatus = status;
    },
  });
  await invalidIdCoordinator.poll();
  assert.equal(invalidIdStatus?.ok, false);
  assert.equal(invalidIdStatus?.code, "V3_APPLICATION_INVALID_SNAPSHOT");
  invalidIdCoordinator.dispose();
});
