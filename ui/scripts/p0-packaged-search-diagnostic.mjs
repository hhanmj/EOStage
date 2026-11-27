import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

import { chromium } from "playwright-core";

const endpoint = process.env.P0_CDP_ENDPOINT ?? "http://127.0.0.1:9330";
const evidencePath = process.env.P0_EVIDENCE_PATH;
if (!evidencePath || !path.isAbsolute(evidencePath)) {
  throw new Error("P0_EVIDENCE_PATH must be an absolute path");
}

const browser = await chromium.connectOverCDP(endpoint);
const context = browser.contexts()[0];
if (!context) throw new Error("packaged WebView2 context was not exposed over CDP");
const page = context.pages()[0] ?? await context.waitForEvent("page");
await page.waitForLoadState("domcontentloaded");
await page.getByLabel("影像数量").waitFor({ state: "visible", timeout: 60_000 });

await page.addInitScript(() => {
  const originalFetch = globalThis.fetch.bind(globalThis);
  const records = [];
  globalThis.__P0_PACKAGED_DIAGNOSTIC__ = records;

  const safeString = (value) => {
    if (typeof value !== "string") return null;
    const bounded = value.slice(0, 600);
    if (/https?:\/\/|[A-Za-z]:[\\/]|authorization|password|secret|token/i.test(bounded)) return null;
    return bounded;
  };
  const safeError = (error) => ({
    value_type: error === null ? "null" : typeof error,
    code: safeString(error?.code),
    message_key: safeString(error?.message_key),
    context: safeString(error?.context),
    reason: safeString(error?.reason),
    message: safeString(error?.message),
    diagnostic: error?.diagnostic && typeof error.diagnostic === "object"
      ? {
          operation: safeString(error.diagnostic.operation),
          dataset: safeString(error.diagnostic.dataset),
          search_route: safeString(error.diagnostic.search_route),
          invalid_request_reason: safeString(error.diagnostic.invalid_request_reason),
        }
      : null,
  });
  const summarizeRequest = (args) => {
    const request = args?.request;
    const query = request?.search?.query ?? {};
    return {
      request_id: typeof request?.request_id === "string" ? request.request_id : null,
      dataset: typeof query.dataset === "string" ? query.dataset : null,
      search_route: "legacy_asf_search",
      query_shape: Object.keys(query).sort(),
      aoi_state: query.aoi_geojson ? "geojson" : query.bbox ? "bbox" : query.aoi_wkt ? "wkt" : "none",
      date_range: { start: query.start ?? null, end: query.end ?? null },
      filter_state: {
        platforms: Array.isArray(query.platforms) ? query.platforms : [],
        processing_levels: Array.isArray(query.processing_levels) ? query.processing_levels : [],
        beam_modes: Array.isArray(query.beam_modes) ? query.beam_modes : [],
        polarizations: Array.isArray(query.polarizations) ? query.polarizations : [],
        flight_direction: query.flight_direction ?? null,
        look_direction: query.look_direction ?? null,
        track_range: query.track_range ?? null,
        frame_range: query.frame_range ?? null,
      },
      max_results: query.max_results ?? null,
    };
  };

  const bodyText = async (input, init) => {
    const body = init?.body;
    if (typeof body === "string") return body;
    if (body instanceof Uint8Array) return new TextDecoder().decode(body);
    if (body instanceof ArrayBuffer) return new TextDecoder().decode(new Uint8Array(body));
    if (input instanceof Request) return input.clone().text();
    return "";
  };
  globalThis.fetch = async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (!url.includes("application_sar_search_and_freeze")) {
      return originalFetch(input, init);
    }
    let args = {};
    try {
      args = JSON.parse(await bodyText(input, init));
    } catch {
      args = {};
    }
    const record = {
      command: "application_sar_search_and_freeze",
      request: summarizeRequest(args),
      started_at_unix_ms: Date.now(),
      outcome: "pending",
    };
    records.push(record);
    try {
      const response = await originalFetch(input, init);
      record.finished_at_unix_ms = Date.now();
      record.outcome = response.headers.get("Tauri-Response") === "ok" ? "ok" : "error";
      if (record.outcome === "error") {
        let error = null;
        try {
          error = JSON.parse(await response.clone().text());
        } catch {
          error = null;
        }
        record.error = safeError(error);
      }
      return response;
    } catch (error) {
      record.outcome = "error";
      record.finished_at_unix_ms = Date.now();
      record.error = safeError(error);
      throw error;
    }
  };
});
await page.reload({ waitUntil: "domcontentloaded" });
await page.getByLabel("影像数量").waitFor({ state: "visible", timeout: 60_000 });
const skipTour = page.getByRole("button", { name: "跳过" });
if (await skipTour.count()) await skipTour.click();

for (const input of await page.locator('input[aria-label*="Path"], input[aria-label*="Relative Orbit"], input[aria-label*="Frame"]').all()) {
  if (await input.isEnabled()) await input.fill("");
}
await page.getByLabel("影像数量").fill("20");
const datasetSelect = page.locator('[data-tour="asf-filter"] select').first();
await datasetSelect.selectOption("SENTINEL-1");

await page.getByRole("button", { name: "检索并导入" }).click();
await page.waitForFunction(() => {
  const record = (globalThis.__P0_PACKAGED_DIAGNOSTIC__ ?? []).at(-1);
  return record && record.outcome !== "pending";
}, undefined, { timeout: 120_000 });
await page.waitForTimeout(750);

const records = await page.evaluate(() => structuredClone(globalThis.__P0_PACKAGED_DIAGNOSTIC__ ?? []));
const visibleFailure = await page.locator("body").evaluate((body) => (
  body.innerText
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => /操作失败|当前请求参数无效|诊断日志/.test(line))
    .slice(0, 8)
));
const evidence = {
  schema_version: 1,
  evidence_kind: "known_bad_packaged_tauri_sar_request",
  captured_at_utc: new Date().toISOString(),
  records,
  visible_failure: visibleFailure,
  geometry_and_credentials_redacted: true,
  downloads_started: false,
};
await mkdir(path.dirname(evidencePath), { recursive: true });
await writeFile(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`, "utf8");
await browser.close();
