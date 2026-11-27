import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

import { chromium } from "playwright-core";

const endpoint = process.env.P0_CDP_ENDPOINT;
const evidencePath = process.env.P0_EVIDENCE_PATH;
const processStartedAt = Number(process.env.P0_PROCESS_START_UNIX_MS ?? "0");
if (!endpoint) throw new Error("P0_CDP_ENDPOINT is required");
if (!evidencePath || !path.isAbsolute(evidencePath)) {
  throw new Error("P0_EVIDENCE_PATH must be an absolute path");
}
if (!Number.isFinite(processStartedAt) || processStartedAt <= 0) {
  throw new Error("P0_PROCESS_START_UNIX_MS must be a positive timestamp");
}

const deadline = Date.now() + 60_000;
let browser = null;
let lastError = null;
while (!browser && Date.now() < deadline) {
  try {
    browser = await chromium.connectOverCDP(endpoint);
  } catch (error) {
    lastError = error;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}
if (!browser) throw lastError ?? new Error("packaged WebView2 CDP endpoint did not become ready");
const context = browser.contexts()[0];
if (!context) throw new Error("packaged WebView2 context was not exposed over CDP");
const page = context.pages()[0] ?? await context.waitForEvent("page");
await page.getByLabel("影像数量").waitFor({ state: "visible", timeout: 60_000 });
const uiReadyAt = Date.now();
const evidence = {
  schema_version: 1,
  evidence_kind: "packaged_tauri_process_launch_to_ui_ready",
  captured_at_utc: new Date().toISOString(),
  process_started_at_unix_ms: processStartedAt,
  ui_ready_at_unix_ms: uiReadyAt,
  process_launch_to_ui_ready_ms: uiReadyAt - processStartedAt,
  readiness_marker: "visible input aria-label=影像数量",
};
await mkdir(path.dirname(evidencePath), { recursive: true });
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
await browser.close();
