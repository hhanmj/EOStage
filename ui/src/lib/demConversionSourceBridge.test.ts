import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopDemConversionRequest,
  type DesktopShellBridge,
} from "./desktopShellBridge.ts";
import {
  createTauriDesktopShellBridge,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";

let sequence = 0;

function tauriDependencies(result: unknown, calls: Array<[string, unknown]>): TauriBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return result;
    },
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

test("strict Tauri DEM source resolver accepts only one fixed scalar response", async () => {
  const request = {
    region_id: "region-dem",
    dataset: "COP30",
    provider: "COPERNICUS_AWS",
    resource_id: "dem.cop30.v1",
    capability_id: "capability.dem.cop30.aws-download.v1",
  };
  const source = {
    task_id: "archived-dem-source",
    revision: 7,
    input_path: "D:\\custom-source\\cop30.tif",
    output_root: "D:\\custom-output",
    ...request,
  };
  const calls: Array<[string, unknown]> = [];
  const bridge = createTauriDesktopShellBridge(
    tauriDependencies(source, calls),
    "tauri://localhost/",
    false,
  );
  assert.deepEqual(await bridge.application.resolveDemConversionSource?.(request), {
    ok: true,
    source,
  });
  assert.deepEqual(calls, [["application_resolve_dem_conversion_source", { request }]]);

  const missing = createTauriDesktopShellBridge(
    tauriDependencies(null, []),
    "tauri://localhost/",
    false,
  );
  assert.deepEqual(await missing.application.resolveDemConversionSource?.(request), {
    ok: true,
    source: null,
  });

  for (const invalid of [
    { ...source, revision: -1 },
    { ...source, snapshot_json: "{}" },
    { ...source, task_id: "x".repeat(129) },
    { ...source, input_path: "" },
    { ...source, output_root: "x".repeat(4097) },
    [{ ...source }],
  ]) {
    const rejected = createTauriDesktopShellBridge(
      tauriDependencies(invalid, []),
      "tauri://localhost/",
      false,
    );
    assert.equal((await rejected.application.resolveDemConversionSource?.(request))?.ok, false);
  }

  const identityDrift = createTauriDesktopShellBridge(
    tauriDependencies({ ...source, dataset: "COP90" }, []),
    "tauri://localhost/",
    false,
  );
  assert.equal((await identityDrift.application.resolveDemConversionSource?.(request))?.ok, false);

  for (const invalidRegion of ["", " region-dem", "region-dem\n", "x".repeat(129)]) {
    const invalidCalls: Array<[string, unknown]> = [];
    const rejected = createTauriDesktopShellBridge(
      tauriDependencies(source, invalidCalls),
      "tauri://localhost/",
      false,
    );
    assert.equal((await rejected.application.resolveDemConversionSource?.({
      ...request,
      region_id: invalidRegion,
    }))?.ok, false);
    assert.equal(invalidCalls.length, 0, "invalid region must not cross the Tauri invoke boundary");
  }
  for (const field of ["dataset", "provider", "resource_id", "capability_id"] as const) {
    const invalidCalls: Array<[string, unknown]> = [];
    const rejected = createTauriDesktopShellBridge(
      tauriDependencies(source, invalidCalls),
      "tauri://localhost/",
      false,
    );
    assert.equal((await rejected.application.resolveDemConversionSource?.({
      ...request,
      [field]: "",
    }))?.ok, false);
    assert.equal(invalidCalls.length, 0, `invalid ${field} must not invoke Tauri`);
  }
});

test("strict Tauri DEM identity setter confirms only the exact bounded scalar mutation", async () => {
  const identity = {
    dataset: "COP90",
    provider: "OPENTOPOGRAPHY",
    updated_at: "2026-08-17T09:10:11Z",
  };
  for (const updatedAt of [
    "2026-08-17T09:10:11Z",
    "2026-08-17T09:10:11.1Z",
    "2026-08-17T09:10:11.01Z",
    "2026-08-17T09:10:11.12Z",
    "2026-08-17T09:10:11.123Z",
  ]) {
    const canonicalIdentity = { ...identity, updated_at: updatedAt };
    const calls: Array<[string, unknown]> = [];
    const bridge = createTauriDesktopShellBridge(
      tauriDependencies(canonicalIdentity, calls),
      "tauri://localhost/",
      false,
    );
    assert.deepEqual(await bridge.application.setDemSourceIdentity?.(canonicalIdentity), {
      ok: true,
      identity: canonicalIdentity,
    });
    assert.deepEqual(calls, [["application_set_dem_source_identity", { identity: canonicalIdentity }]]);
  }

  for (const invalid of [
    { ...identity, dataset: "cop90" },
    { ...identity, provider: " OPENTOPOGRAPHY" },
    { ...identity, updated_at: "2026-08-17T09:10:11.000Z" },
    { ...identity, updated_at: "2026-08-17T09:10:11.010Z" },
    { ...identity, updated_at: "2026-08-17T09:10:11.120Z" },
    { ...identity, updated_at: "2026-08-17T09:10:11.1234Z" },
    { ...identity, updated_at: "2026-08-17T09:10:11+00:00" },
    { ...identity, updated_at: "2026-08-17T09:10:11Z\n" },
    { ...identity, extra: [] },
  ]) {
    const invalidCalls: Array<[string, unknown]> = [];
    const invalidBridge = createTauriDesktopShellBridge(
      tauriDependencies(identity, invalidCalls),
      "tauri://localhost/",
      false,
    );
    assert.equal((await invalidBridge.application.setDemSourceIdentity?.(invalid as typeof identity))?.ok, false);
    assert.equal(invalidCalls.length, 0);
  }
  const drifted = createTauriDesktopShellBridge(
    tauriDependencies({ ...identity, provider: "COPERNICUS_AWS" }, []),
    "tauri://localhost/",
    false,
  );
  assert.equal((await drifted.application.setDemSourceIdentity?.(identity))?.ok, false);
});

function emptyCoreRestore(demIdentity: Record<string, unknown> | null = null) {
  return {
    ok: true as const,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
    core: {
      readiness: { schema_version: 11, ready: true },
      workspace_context: {
        workspace_id: "workspace-dem",
        workspace_root: "C:\\InSAR",
        workspace_name: "Workspace",
        project_id: "project-dem",
        project_name: "Project",
        region_id: "region-dem",
        region_name: "Region",
      },
      region_aoi: null,
      region_aois: [],
      dem_source_identity: demIdentity,
      tasks: [],
      legacy_task_warnings: [],
      workbench: null,
    },
  };
}

async function importBridgeForArchivedSource(calls: {
  core: boolean[];
  resolvedRequests: Array<Record<string, unknown>>;
  handoffs: DesktopDemConversionRequest[];
}) {
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
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        coreRestore: async (includeTerminal = false) => {
          calls.core.push(includeTerminal);
          return emptyCoreRestore({
            dataset: "COP90",
            provider: "OPENTOPOGRAPHY",
            updated_at: "2026-08-17T09:10:11Z",
          });
        },
        resolveDemConversionSource: async (request: Record<string, unknown>) => {
          calls.resolvedRequests.push(request);
          return {
            ok: true,
            source: {
              task_id: "archived-dem-source",
              revision: 7,
              region_id: "region-dem",
              dataset: "COP90",
              provider: "OPENTOPOGRAPHY",
              resource_id: "dem.cop90.v1",
              capability_id: "capability.dem.cop90.opentopography-download.v1",
              input_path: "D:\\custom-source\\cop90.tif",
              output_root: "D:\\custom-output",
            },
          };
        },
        handoffDemConversion: async (request: DesktopDemConversionRequest) => {
          calls.handoffs.push(request);
          return emptyCoreRestore();
        },
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?dem-conversion-source-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

test("formal DEM conversion resolves an archived source when active-only CoreRestore is empty", async () => {
  const calls = {
    core: [] as boolean[],
    resolvedRequests: [] as Array<Record<string, unknown>>,
    handoffs: [] as DesktopDemConversionRequest[],
  };
  const bridge = await importBridgeForArchivedSource(calls);
  try {
    const result = await bridge.runDemConversion("Z:\\ui-default-must-not-win");
    assert.equal(result.ok, true);
    assert.deepEqual(calls.core, [false], "context is one active-only restore; source lookup is scalar");
    assert.deepEqual(calls.resolvedRequests, [{
      region_id: "region-dem",
      dataset: "COP90",
      provider: "OPENTOPOGRAPHY",
      resource_id: "dem.cop90.v1",
      capability_id: "capability.dem.cop90.opentopography-download.v1",
    }]);
    assert.equal(calls.handoffs.length, 1);
    assert.equal(calls.handoffs[0].source_task_id, "archived-dem-source");
    assert.equal(calls.handoffs[0].region_id, "region-dem");
    assert.equal(calls.handoffs[0].input_path, "D:\\custom-source\\cop90.tif");
    assert.equal(calls.handoffs[0].output_root, "D:\\custom-output");
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("same-session DEM selection and failed mutation keep Store identity authoritative for conversion", async () => {
  const NativeDate = globalThis.Date;
  const fixedNow = NativeDate.parse("2026-08-17T09:10:11.010Z");
  class FixedDate extends NativeDate {
    constructor(value?: string | number) {
      super(value === undefined ? fixedNow : value);
    }
    static now() { return fixedNow; }
  }
  globalThis.Date = FixedDate as DateConstructor;
  let persisted: Record<string, unknown> | null = null;
  let rejectNext = false;
  const selected: Array<Record<string, unknown>> = [];
  const resolved: Array<Record<string, unknown>> = [];
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
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        setDemSourceIdentity: async (identity: Record<string, unknown>) => {
          selected.push(identity);
          if (rejectNext) {
            rejectNext = false;
            return { ok: false, error: "rejected", code: "V3_APPLICATION_INVALID_REQUEST" };
          }
          persisted = { ...identity };
          return { ok: true, identity };
        },
        coreRestore: async () => emptyCoreRestore(persisted),
        resolveDemConversionSource: async (request: Record<string, unknown>) => {
          resolved.push(request);
          return {
            ok: true,
            source: {
              task_id: "persisted-source",
              revision: 2,
              ...request,
              input_path: "D:\\persisted\\cop90.tif",
              output_root: "D:\\persisted-output",
            },
          };
        },
        handoffDemConversion: async () => emptyCoreRestore(persisted),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    const bridge = await import(`./bridge.ts?dem-selection-session-${sequence}`);
    assert.deepEqual(await bridge.setDemDataset("COP90", "OPENTOPOGRAPHY"), {
      ok: true,
      dataset: "COP90",
      provider: "OPENTOPOGRAPHY",
    });
    assert.equal(selected[0].updated_at, "2026-08-17T09:10:11.01Z");
    assert.equal((await bridge.runDemConversion()).ok, true);
    assert.equal(resolved[0].dataset, "COP90");
    assert.equal(resolved[0].provider, "OPENTOPOGRAPHY");

    rejectNext = true;
    assert.equal((await bridge.setDemDataset("COP30", "COPERNICUS_AWS")).ok, false);
    assert.equal((await bridge.runDemConversion()).ok, true);
    assert.equal(resolved[1].dataset, "COP90", "failed set must not replace restored Store identity");
    assert.equal(resolved[1].provider, "OPENTOPOGRAPHY");
    assert.equal(selected.length, 2);
  } finally {
    globalThis.Date = NativeDate;
    clearTestDesktopShellBridge();
    hooks.deregister();
  }
});

test("persisted DEM provider/dataset mismatch fails closed instead of using the default", async () => {
  let persisted: Record<string, unknown> | null = null;
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
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        coreRestore: async () => emptyCoreRestore(persisted),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    const bridge = await import(`./bridge.ts?dem-selection-mismatch-${sequence}`);
    assert.deepEqual(await bridge.getContext(), {
      ok: true,
      workspace: { workspace_id: "workspace-dem", root: "C:\\InSAR", name: "Workspace" },
      project: { project_id: "project-dem", name: "Project", safe_name: "Project", root: "C:\\InSAR\\Project" },
      region: {
        region_id: "region-dem",
        name: "Region",
        safe_name: "Region",
        root: "C:\\InSAR\\Project\\Region",
        has_aoi: false,
        bbox: null,
        aoi_geojson: undefined,
        aoi_source: null,
        aoi_name: null,
        scene_count: 0,
      },
      dem_dataset: "COP30",
      dem_provider: "COPERNICUS_AWS",
    });
    persisted = {
      dataset: "COP90",
      provider: "LOCAL_FILE",
      updated_at: "2026-08-17T09:10:11Z",
    };
    const context = await bridge.getContext();
    assert.equal(context.ok, false);
    assert.equal(context.code, "V3_APPLICATION_INVALID_SNAPSHOT");
  } finally {
    clearTestDesktopShellBridge();
    hooks.deregister();
  }
});
