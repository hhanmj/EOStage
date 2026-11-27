import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";
import type { CoreTaskSummaryProjection } from "./coreRestoreProjection.ts";
import { parseCoreTaskSummary } from "./coreRestoreProjection.ts";
import { projectDownloadStatusFromCoreTask } from "./downloadStatusProjection.ts";

const archiveModule = { exports: {} as Record<string, any> };
runInNewContext(ts.transpileModule(
  readFileSync(new URL("./terminalTaskArchiver.ts", import.meta.url), "utf8"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } },
).outputText, { exports: archiveModule.exports, require: () => ({}) });
const { TerminalTaskArchiveRuntime } = archiveModule.exports;

const source = readFileSync(new URL("./bridge.ts", import.meta.url), "utf8");
const parsed = ts.createSourceFile("bridge.ts", source, ts.ScriptTarget.Latest, true);
function productionFunction(name: string, dependencies: Record<string, unknown>) {
  const node = parsed.statements.find((item) => ts.isFunctionDeclaration(item) && item.name?.text === name);
  assert.ok(node);
  const code = ts.transpileModule(node.getText(parsed).replace(/^export\s+/, ""), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
  }).outputText;
  return runInNewContext(`${code}\n${name}`, dependencies);
}

const joinDemPath = productionFunction("joinDemPath", {});
const standaloneDemSourceDatum = productionFunction("standaloneDemSourceDatum", {});
const standaloneDemStem = productionFunction("standaloneDemStem", {});
const formalDemDependencies = {
  demSourceStem: () => "fixture",
  joinDemPath,
  MOCK_BBOX: {},
  standaloneDemSourceDatum,
  standaloneDemStem,
};

function task(kind: string | null, id: string): CoreTaskSummaryProjection {
  return {
    task_id: id, batch_id: `batch-${id}`, provider_id: kind === "dem" ? "compat.dem" : "asf.sentinel1",
    dataset_id: kind === "dem" ? "COP90" : "SENTINEL-1", created_at: "2026-09-07T12:00:00Z",
    status: "running", revision: 1, updated_at: "2026-09-07T12:00:03Z",
    selected_count: 1, checked_count: 1, active_count: 1, paused_count: 0, queued_count: 0,
    completed_count: 0, failed_count: 0, skipped_count: 0, cancelled_count: 0,
    terminal_bytes: 0, active_bytes: 120, other_bytes: 0, transferred_bytes: 120,
    total_bytes: null, bytes_per_second: 40, transfer_active_millis: 3000,
    active_item: { item_id: `${id}-item`, bytes_completed: 120, total_bytes: null },
    output_root: "C:/fixture", layout: "direct", actionable: true,
    operation: kind ? { operation_id: `${id}-operation`, kind, phase: "download", region_id: "fixture", approved_worker_operation: `${kind}_download` } : null,
  };
}

test("DEM status projects measured Store bytes and rate without inventing unknown totals", () => {
  const project = productionFunction("formalDemTaskStatus", formalDemDependencies);
  const input = task("dem", "dem-1");
  const status = project(input, { root: "C:/fixture", bbox: {} }, 1, 0);
  assert.equal(status.done_bytes, 120);
  assert.equal(status.bytes_per_second, 40);
  assert.equal(status.elapsed_seconds, 3);
  assert.equal(status.total_bytes, undefined);
  input.total_bytes = 240;
  assert.equal(project(input, { bbox: {} }, 1, 0).total_bytes, 240);
});

test("formal DEM status uses the frozen scientific source instead of compat.dem for resume policy", () => {
  const project = productionFunction("formalDemTaskStatus", formalDemDependencies);
  const input = task("dem", "dem-aws");
  assert.ok(input.operation);
  input.operation.source_provider = "COPERNICUS_AWS";
  const status = project(input, { root: "C:/fixture", bbox: {} }, 1, 0);
  assert.equal(status.task_snapshot.provider, "COPERNICUS_AWS");
  assert.equal(status.resume_supported, true);
  assert.equal(status.task_snapshot.resume_supported, true);
});

test("SAR list and aggregate exclude DEM tasks without changing the shared Store snapshot", async () => {
  const original = [task(null, "sar-1"), task("dem", "dem-1")];
  const read = productionFunction("getDownloadStatus", {
    isTauriDesktopHost: () => true,
    restoreApplicationCore: async () => ({ ok: true, core: { tasks: original } }),
    projectDownloadStatusFromCoreTask,
  });
  const result = await read();
  assert.equal(result.ok, true);
  assert.equal(result.asf_tasks.length, 1);
  assert.equal(result.asf_tasks[0].task_id, "sar-1");
  assert.equal(result.transferred_bytes, 120);
  assert.equal(result.bytes_per_second, 40);
  assert.equal(original.length, 2);
});

test("native DEM network progress survives tile completion and differs from crop bytes", () => {
  const project = productionFunction("formalDemTaskStatus", formalDemDependencies);
  const input = task("dem", "dem-network");
  input.network_bytes_received = 4096;
  input.total_bytes = 120; // raster/output telemetry must not become a network total
  const parsed = parseCoreTaskSummary(input);
  assert.ok(parsed);
  assert.equal(project(parsed, { bbox: {} }, 1, 0).done_bytes, 4096);
  assert.equal(project(parsed, { bbox: {} }, 1, 0).total_bytes, undefined);
  input.network_bytes_received = 8192;
  input.status = "completed";
  assert.equal(project(input, { bbox: {} }, 1, 0).done_bytes, 8192);
  assert.equal(project(input, { bbox: {} }, 1, 0).state, "finished");
  for (const invalid of [-1, NaN, Infinity, 1.5, "8192"]) {
    assert.equal(parseCoreTaskSummary({ ...input, network_bytes_received: invalid }), null);
  }
});

test("completed DEM retains revision and reaches the shared durable archiver after leaving SAR cards", async () => {
  const project = productionFunction("formalDemTaskStatus", formalDemDependencies);
  const input = { ...task("dem", "dem-completed"), status: "completed", revision: 7, completed_count: 1 };
  const status = project(input, { root: "C:/fixture", bbox: {} }, 1, 0);
  assert.equal(status.state, "finished");
  assert.equal(status.revision, 7);
  assert.equal(status.dataset, "COP90");
  assert.equal(status.task_snapshot.created_at, Date.parse(input.created_at) / 1000);
  const calls: string[] = [];
  const archived = new TerminalTaskArchiveRuntime({
    archive: async (id: string, revision: number) => { calls.push(`${id}:${revision}`); return { ok: true }; },
    onSuccess: () => {}, onError: () => assert.fail("unexpected archive error"),
  });
  archived.activate();
  archived.publish(true, [status], []);
  await Promise.resolve();
  assert.deepEqual(calls, ["dem-completed:7"]);
  archived.publish(true, [status], ["dem-completed:7"]);
  assert.equal(calls.length, 1);
  archived.deactivate();
});

test("DEM cancellation and failure are not converted into successful completion", () => {
  const project = productionFunction("formalDemTaskStatus", formalDemDependencies);
  for (const state of ["queued", "running", "paused", "cancelled", "failed", "interrupted"]) {
    assert.equal(project({ ...task("dem", state), status: state }, { bbox: {} }, 1, 0).state, state);
  }
});
