import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";

// Execute the current production request builder; only external state/IPC is stubbed.
const source = readFileSync(new URL("./bridge.ts", import.meta.url), "utf8");
const parsed = ts.createSourceFile("bridge.ts", source, ts.ScriptTarget.Latest, true);
const names = new Set(["stableUiNodeId", "startTauriDemForCurrentAoi"]);
const functions = parsed.statements.filter((node) => ts.isFunctionDeclaration(node) && node.name && names.has(node.name.text));
assert.equal(functions.length, names.size);
const executable = ts.transpileModule(functions.map((node) => node.getText(parsed)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None },
}).outputText;

test("native DEM requests retain distinct IDs with long Chinese AOI names and repeated submissions", async () => {
  const requests: Record<string, unknown>[] = [];
  let name = "测试区域".repeat(30);
  let geometry = { type: "Polygon", coordinates: [[[100, 30], [101, 30], [101, 31], [100, 30]]] };
  const builder = runInNewContext(`${executable}\nstartTauriDemForCurrentAoi`, {
    getContext: async () => ({ ok: true }),
    exactOperationAoi: async () => ({ ok: true, display_name: name, geometry }),
    formalDemIdentity: (dataset: string, provider: string) => ({ dataset, provider }),
    standaloneDemOutputRoot: (root: string) => root,
    recordValue: (value: unknown) => value,
    persistedDemTaskConcurrency: async () => 1,
    createApplicationStandaloneDemTask: async (request: Record<string, unknown>) => { requests.push(request); return { ok: true }; },
  });
  await builder("C:/fixture/first", "COP90", false, "COPERNICUS_AWS");
  await builder("C:/fixture/second", "COP90", false, "COPERNICUS_AWS");
  name += "新区";
  geometry = { type: "Polygon", coordinates: [[[102, 30], [103, 30], [103, 31], [102, 30]]] };
  await builder("C:/fixture/third", "COP90", true, "COPERNICUS_AWS");
  await builder("C:/fixture/fourth", "COP30", false, "COPERNICUS_AWS");
  assert.equal(new Set(requests.map((request) => request.task_id)).size, 4);
  for (const request of requests) assert.match(String(request.task_id), /^[a-z0-9-]{1,64}$/);
  assert.notDeepEqual(requests[0].aoi_geojson, requests[2].aoi_geojson);
  assert.equal(requests[0].aoi_name, "测试区域".repeat(30));
  assert.equal(requests[3].dataset, "COP30");
});
