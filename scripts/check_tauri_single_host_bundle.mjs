import assert from "node:assert/strict";
import { access, readdir, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { assertDesktopAtlasCdnCspConsistency } from "./check_atlas_cdn_csp.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function filesUnder(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesUnder(absolute)));
    else files.push(absolute);
  }
  return files.sort();
}

async function graphText(directory) {
  const files = await filesUnder(directory);
  return (await Promise.all(files.map((file) => readFile(file, "utf8")))).join("\n");
}

const directory = path.join(root, "ui", "dist-tauri");
await access(path.join(directory, "index.html"));
const outputFiles = await filesUnder(directory);
assert.ok(
  outputFiles.some((file) => path.basename(file) === "index.html"),
  "missing Tauri production HTML entry: index.html",
);
assert.ok(
  !outputFiles.some((file) => path.basename(file) === "tauri.html"),
  "Tauri production output must expose index.html, not tauri.html",
);
const text = await graphText(directory);
const [buildScript, workerRuntime, componentContract] = await Promise.all([
  readFile(path.join(root, "src-tauri", "build.rs"), "utf8"),
  readFile(path.join(root, "src-tauri", "src", "compat_worker.rs"), "utf8"),
  readFile(path.join(root, "contracts", "compat-worker-component.v1.json"), "utf8"),
]);
const contract = JSON.parse(componentContract);
for (const field of [
  "component_id",
  "protocol",
  "relative_path",
  "executable_name",
  "version",
  "sha256",
  "supported_operations",
  "offline_only",
  "store_access",
  "credential_access",
  "download_queue_access",
  "lifecycle_owner",
  "task_id_owner",
]) {
  assert.ok(Object.hasOwn(contract, field), `missing Worker component contract field: ${field}`);
}
assert.equal(contract.lifecycle_owner, "tauri");
assert.equal(contract.task_id_owner, "application_service");
assert.doesNotMatch(buildScript, /include_bytes!|EOSTAGE_COMPAT_WORKER_EXE/);
assert.doesNotMatch(workerRuntime, /WORKER_BYTES|extract_embedded_worker|temp_dir\(\)/);
assert.match(workerRuntime, /join\("runtime"\)/);
assert.match(workerRuntime, /join\("compat-worker"\)/);

for (const marker of [
  "Workbench",
  "application_catalog_search_and_freeze",
  "application_aoi_import",
  "application_aoi_normalize",
  "application_admin_boundary_search",
  "application_gacos_plan",
  "application_gacos_dry_run",
  "application_report_generate",
]) {
  assert.match(text, new RegExp(marker), `missing single-host marker: ${marker}`);
}
for (const forbidden of [
  "application_service_restore",
  "compatibility_worker_call",
  "m1Snapshot",
]) {
  assert.doesNotMatch(text, new RegExp(forbidden), `forbidden normal production marker: ${forbidden}`);
}
for (const marker of [
  "tauriFoundationMain",
  "TauriFoundationShell",
  "tauriM1Main",
  "Browser Fixture",
  "Desktop shell bridge is unavailable",
  "Desktop application service is unavailable",
  "Desktop compatibility Worker is unavailable",
  "pywebview application service route is not enabled",
]) {
  assert.doesNotMatch(text, new RegExp(marker), `forbidden formal bundle marker: ${marker}`);
}

for (const forbiddenAtlas of [
  "\"/atlas-cdn/v2/",
  "'/atlas-cdn/v2/",
]) {
  assert.ok(
    !text.includes(forbiddenAtlas),
    `unconfigured Tauri bundle must not emit hardcoded ${forbiddenAtlas} requests`
  );
}
assert.ok(
  !outputFiles.some((file) => /(?:^|[\\/])atlas-cdn(?:-pack)?(?:[\\/]|$)/.test(file)),
  "Tauri dist-tauri must not bundle atlas-cdn-pack assets"
);
assertDesktopAtlasCdnCspConsistency({
  repoRoot: root,
  rawBase: process.env.VITE_EO_ATLAS_CDN_BASE_URL ?? "",
  mode: "build",
});

console.log("Tauri single-host bundle graph is business-capable and PyWebView-free.");
