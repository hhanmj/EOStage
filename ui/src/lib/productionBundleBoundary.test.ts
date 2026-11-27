import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("production App excludes legacy Nextgen prototype and keeps dev harness lazy", async () => {
  const source = await readFile(new URL("../App.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(source, /NextgenWorkbench/);
  assert.match(source, /import\.meta\.env\.DEV/);
  assert.match(source, /lazy\(\(\)\s*=>\s*import\([^)]*DesktopHarness/);
});
