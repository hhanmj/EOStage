import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  initialHashToWrite,
  parseShellRoute,
  resolveInitialRoute,
} from "../shell/api/shellRoutes.ts";
import {
  configureUiSmoke,
  UI_SMOKE_SHELL_HASH,
  uiSmokeReportForTest,
  uiSmokeStartupHash,
} from "./uiSmoke.ts";

// The packaged phase_b report requires the Leaflet map checks. In the W5 shell
// only the workbench page mounts the map at startup; the default agent page
// keeps it in a closed side panel, so the smoke must start on #/workbench.

test("smoke startup moves a fresh or agent hash onto the workbench route", () => {
  for (const hash of ["", "#/agent", "#/history", "#garbage", "#/downloads"]) {
    assert.equal(uiSmokeStartupHash(hash), UI_SMOKE_SHELL_HASH, hash);
  }
  assert.equal(uiSmokeStartupHash(UI_SMOKE_SHELL_HASH), null);
});

test("the smoke hash wins over a persisted route and is not rewritten by the shell", () => {
  assert.equal(parseShellRoute(UI_SMOKE_SHELL_HASH), "workbench");
  for (const stored of [null, "agent", "history", "downloads"]) {
    assert.equal(resolveInitialRoute(UI_SMOKE_SHELL_HASH, stored), "workbench");
  }
  assert.equal(initialHashToWrite(UI_SMOKE_SHELL_HASH, "workbench"), null);
  // Without the smoke hash a fresh profile lands on the agent page.
  assert.equal(resolveInitialRoute("", null), "agent");
});

function rustReportFields(): string[] {
  const source = readFileSync(
    new URL("../../../src-tauri/src/ui_smoke.rs", import.meta.url),
    "utf8",
  );
  const body = source.match(/pub struct UiSmokeReport \{([\s\S]*?)\n\}/);
  assert.ok(body, "UiSmokeReport struct not found");
  return [...body[1].matchAll(/^\s*pub (\w+):/gm)].map((match) => match[1]);
}

test("the browser report and Rust report name the same fields", () => {
  configureUiSmoke(false);
  const browser = Object.keys(uiSmokeReportForTest()).sort();
  assert.ok(browser.includes("leaflet_initialized"));
  assert.deepEqual(rustReportFields().sort(), browser);
});
