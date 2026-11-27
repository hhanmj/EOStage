import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, readFile, realpath, stat, readdir } from "node:fs/promises";
import { resolve, join, relative, isAbsolute, parse } from "node:path";
import { pathToFileURL } from "node:url";

export class NativeGisPackageError extends Error {
  constructor(code, detail) { super(`${code}: ${detail}`); this.code = code; }
}
function requireCondition(condition, code, detail) {
  if (!condition) throw new NativeGisPackageError(code, detail);
}
function safeRelative(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 512
    && !/[\\:<>"|?*\x00-\x1f\x7f]/.test(value) && !isAbsolute(value)
    && value.split("/").every((part) => part && part !== "." && part !== ".."
      && !/[. ]$/.test(part) && !/^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(part));
}
async function ownedRoot(root) {
  const anchor = parse(root).root;
  let current = anchor;
  for (const part of relative(anchor, root).split(/[\\/]/).filter(Boolean)) {
    current = join(current, part);
    let item;
    try { item = await lstat(current); } catch {
      throw new NativeGisPackageError("NATIVE_GIS_PACKAGE_REQUIRED", "runtime/gis is missing");
    }
    requireCondition(!item.isSymbolicLink(), "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime root or ancestor is linked");
    requireCondition(item.isDirectory(), "NATIVE_GIS_PACKAGE_REQUIRED", "runtime path is not a directory");
  }
}
async function packageFileClosure(root, admitted, ignoredDirectories = new Set()) {
  const pending = [""];
  const files = [];
  const seen = new Set();
  let entries = 0;
  while (pending.length) {
    const directory = pending.pop();
    const children = await readdir(join(root, directory), { withFileTypes: true });
    for (const child of children) {
      requireCondition(++entries <= 40000, "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "runtime inventory exceeds the bounded budget");
      const name = directory ? `${directory}/${child.name}` : child.name;
      requireCondition(safeRelative(name) && name.split("/").length <= 32,
        "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime contains an invalid path");
      const item = await lstat(join(root, ...name.split("/")));
      requireCondition(!item.isSymbolicLink(), "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime links are not package-owned files");
      if (item.isDirectory()) {
        if (!ignoredDirectories.has(name.toLowerCase())) pending.push(name);
        continue;
      }
      requireCondition(item.isFile(), "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime contains a non-regular file");
      const key = name.toLowerCase();
      requireCondition(admitted.has(key), "NATIVE_GIS_PACKAGE_UNDECLARED_FILE", name);
      requireCondition(!seen.has(key), "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime paths alias one another");
      seen.add(key); files.push(name);
    }
  }
  requireCondition(seen.size === admitted.size && [...admitted].every((name) => seen.has(name)),
    "NATIVE_GIS_PACKAGE_FILE_MISSING", "declared file set is incomplete");
  return files.sort();
}
async function containedPath(root, name) {
  requireCondition(safeRelative(name), "NATIVE_GIS_PACKAGE_PATH_INVALID", "manifest path is not a bounded relative path");
  const base = await realpath(root);
  const candidate = join(base, ...name.split("/"));
  let current = base;
  for (const part of name.split("/")) {
    current = join(current, part);
    let item;
    try { item = await lstat(current); } catch {
      throw new NativeGisPackageError("NATIVE_GIS_PACKAGE_FILE_MISSING", name);
    }
    requireCondition(!item.isSymbolicLink(), "NATIVE_GIS_PACKAGE_PATH_INVALID", "runtime links are not package-owned files");
  }
  const actual = await realpath(candidate);
  const rel = relative(base, actual);
  requireCondition(rel !== ".." && !rel.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) && !isAbsolute(rel),
    "NATIVE_GIS_PACKAGE_PATH_INVALID", "manifest path escapes the runtime root");
  return candidate;
}
async function readManifest(root, name) {
  const path = await containedPath(root, name);
  const info = await stat(path);
  requireCondition(info.isFile() && info.size > 0 && info.size <= 2 * 1024 * 1024,
    "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", name);
  const raw = await readFile(path);
  let value;
  try { value = JSON.parse(raw.toString("utf8")); } catch {
    throw new NativeGisPackageError("NATIVE_GIS_PACKAGE_MANIFEST_INVALID", name);
  }
  return { value, sha256: createHash("sha256").update(raw).digest("hex") };
}
async function verifyFile(root, entry) {
  requireCondition(entry && Number.isSafeInteger(entry.bytes) && entry.bytes > 0
    && typeof entry.sha256 === "string" && /^[a-fA-F0-9]{64}$/.test(entry.sha256),
    "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "invalid file identity");
  const path = await containedPath(root, entry.relative_path);
  const before = await stat(path);
  requireCondition(before.isFile() && before.size === entry.bytes,
    "NATIVE_GIS_PACKAGE_IDENTITY_MISMATCH", entry.relative_path);
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  const after = await stat(path);
  requireCondition(before.size === after.size && before.mtimeMs === after.mtimeMs
    && hash.digest("hex") === entry.sha256.toLowerCase(),
    "NATIVE_GIS_PACKAGE_IDENTITY_MISMATCH", entry.relative_path);
}

/** Build-time artifact check, not a replacement for loading the native engine. */
export async function verifyNativeGisRuntime(runtimeRoot, verticalGridRoot = null) {
  requireCondition(typeof runtimeRoot === "string" && runtimeRoot.trim().length > 0,
    "NATIVE_GIS_PACKAGE_REQUIRED", "the production desktop requires adjacent runtime/gis; the compatibility Worker cannot replace it");
  const root = resolve(runtimeRoot);
  const gridRoot = verticalGridRoot ? resolve(verticalGridRoot) : join(root, "vertical-grids");
  await ownedRoot(root);
  await ownedRoot(gridRoot);
  const manifest = await readManifest(root, "manifest.json");
  const value = manifest.value;
  requireCondition(value && value.schema_version === 1 && typeof value.runtime_id === "string"
    && value.runtime_id.length > 0 && value.proj_network === "disabled"
    && Array.isArray(value.drivers) && [...value.drivers].sort().join(",") === "ENVI,GTiff"
    && Array.isArray(value.files) && value.files.length > 0 && value.files.length <= 20000,
    "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "native runtime identity/driver/network contract is incomplete");
  const files = new Map();
  for (const entry of value.files) {
    requireCondition(entry && safeRelative(entry.relative_path) && !files.has(entry.relative_path.toLowerCase()),
      "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "duplicate or invalid file path");
    files.set(entry.relative_path.toLowerCase(), entry);
  }
  for (const name of [value.gdal_library, value.proj_library, `${value.proj_data_dir}/proj.db`]) {
    requireCondition(safeRelative(name) && files.has(name.toLowerCase()),
      "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "GDAL, PROJ and proj.db must belong to the verified file closure");
  }
  for (const name of [value.proj_data_dir, value.gdal_data_dir]) {
    const path = await containedPath(root, name);
    requireCondition((await stat(path)).isDirectory(), "NATIVE_GIS_PACKAGE_FILE_MISSING", "runtime data directory missing");
  }
  const grids = await readManifest(gridRoot, "manifest.json");
  requireCondition(Array.isArray(grids.value), "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "vertical grids must be an array");
  const gridFiles = new Map();
  const gridComponents = [];
  for (const model of ["EGM96", "EGM2008"]) {
    const matches = grids.value.filter((entry) => entry?.model === model);
    requireCondition(matches.length === 1 && matches[0].bundled === true && matches[0].production_admitted === true,
      "NATIVE_GIS_PACKAGE_GRID_UNAVAILABLE", `${model} must be bundled and production-admitted`);
    const entry = matches[0];
    requireCondition(safeRelative(entry.filename), "NATIVE_GIS_PACKAGE_PATH_INVALID", "invalid vertical-grid filename");
    const path = entry.filename;
    const key = path.toLowerCase();
    requireCondition(!gridFiles.has(key), "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "vertical models alias one file");
    const outer = files.get(`vertical-grids/${key}`);
    requireCondition(!outer || (outer.bytes === entry.expected_bytes
      && typeof outer.sha256 === "string" && typeof entry.sha256 === "string"
      && outer.sha256.toLowerCase() === entry.sha256.toLowerCase()),
      "NATIVE_GIS_PACKAGE_MANIFEST_INVALID", "outer and vertical manifests disagree on file identity");
    gridFiles.set(key, { relative_path: path, bytes: entry.expected_bytes, sha256: entry.sha256 });
    gridComponents.push({
      component_id: `vertical-grid-${model.toLowerCase()}`,
      model,
      grid_id: entry.grid_id ?? `${model.toLowerCase()}-grid`,
      version: entry.version ?? "",
      relative_path: path,
      bytes: entry.expected_bytes,
      sha256: entry.sha256,
      license: entry.license ?? "",
      coverage: entry.coverage ?? "",
      resolution: entry.resolution ?? "",
    });
  }
  const runtimeFiles = new Map(
    [...files].filter(([key]) => !key.startsWith("vertical-grids/")),
  );
  const runtimeAdmitted = new Set([...runtimeFiles.keys(), "manifest.json"]);
  const runtimePaths = await packageFileClosure(root, runtimeAdmitted, new Set(["vertical-grids"]));
  const gridAdmitted = new Set([...gridFiles.keys(), "manifest.json"]);
  const verticalGridPaths = await packageFileClosure(gridRoot, gridAdmitted);
  for (const entry of runtimeFiles.values()) await verifyFile(root, entry);
  for (const entry of gridFiles.values()) await verifyFile(gridRoot, entry);
  requireCondition((await readManifest(root, "manifest.json")).sha256 === manifest.sha256
    && (await readManifest(gridRoot, "manifest.json")).sha256 === grids.sha256,
    "NATIVE_GIS_PACKAGE_IDENTITY_MISMATCH", "manifest changed during verification");
  await ownedRoot(root);
  await ownedRoot(gridRoot);
  requireCondition(JSON.stringify(await packageFileClosure(root, runtimeAdmitted, new Set(["vertical-grids"]))) === JSON.stringify(runtimePaths),
    "NATIVE_GIS_PACKAGE_IDENTITY_MISMATCH", "runtime file set changed during verification");
  requireCondition(JSON.stringify(await packageFileClosure(gridRoot, gridAdmitted)) === JSON.stringify(verticalGridPaths),
    "NATIVE_GIS_PACKAGE_IDENTITY_MISMATCH", "vertical-grid file set changed during verification");
  const packagePaths = [
    ...runtimePaths,
    ...verticalGridPaths.map((path) => `vertical-grids/${path}`),
  ].sort();
  return { ok: true, runtime_id: value.runtime_id, manifest_sha256: manifest.sha256,
    vertical_manifest_sha256: grids.sha256, files_verified: runtimeFiles.size + gridFiles.size,
    vertical_models: ["EGM96", "EGM2008"],
    package_paths: packagePaths, package_directories: [...new Set([value.proj_data_dir, value.gdal_data_dir])].sort(),
    runtime_component: {
      component_id: "native-gis-runtime",
      install_root: "runtime/gis",
      runtime_id: value.runtime_id,
      manifest_sha256: manifest.sha256,
      package_paths: runtimePaths,
      files_verified: runtimeFiles.size,
    },
    vertical_grid_component: {
      component_id: "vertical-grids",
      install_root: "runtime/vertical-grids",
      manifest_sha256: grids.sha256,
      package_paths: verticalGridPaths,
      grids: gridComponents,
    },
    native_load_verified: false };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    requireCondition(
      (process.argv.length === 4 && process.argv[2] === "--runtime-root")
      || (process.argv.length === 6 && process.argv[2] === "--runtime-root" && process.argv[4] === "--vertical-grid-root"),
      "NATIVE_GIS_PACKAGE_ARGUMENTS_INVALID",
      "expected --runtime-root <path> [--vertical-grid-root <path>]",
    );
    console.log(JSON.stringify(await verifyNativeGisRuntime(
      process.argv[3],
      process.argv.length === 6 ? process.argv[5] : null,
    )));
  } catch (error) {
    console.error(error instanceof NativeGisPackageError ? error.message : "NATIVE_GIS_PACKAGE_CHECK_FAILED: runtime identity could not be verified");
    process.exitCode = 1;
  }
}
