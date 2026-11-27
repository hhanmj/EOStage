import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("the undecorated desktop starts maximized so Windows work area owns the taskbar boundary", () => {
  const config = JSON.parse(
    readFileSync(new URL("../../../src-tauri/tauri.conf.json", import.meta.url), "utf8"),
  ) as { app: { windows: Array<{ decorations?: boolean; maximized?: boolean }> } };
  const main = config.app.windows.find((window) => window.decorations === false);
  assert.ok(main);
  assert.equal(main.maximized, true);
});
