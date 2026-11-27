import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";

import { chromium } from "playwright-core";

const endpoint = process.env.P0_CDP_ENDPOINT ?? "http://127.0.0.1:9222";
const evidencePath = process.env.P0_EVIDENCE_PATH;
const screenshotPath = process.env.P0_SCREENSHOT_PATH;
const processStartedAt = Number(process.env.P0_PROCESS_START_UNIX_MS ?? "0");

if (!evidencePath || !path.isAbsolute(evidencePath)) {
  throw new Error("P0_EVIDENCE_PATH must be an absolute path");
}
if (screenshotPath && !path.isAbsolute(screenshotPath)) {
  throw new Error("P0_SCREENSHOT_PATH must be an absolute path");
}

const browser = await chromium.connectOverCDP(endpoint);
const context = browser.contexts()[0];
if (!context) throw new Error("packaged WebView2 context was not exposed over CDP");
const page = context.pages()[0] ?? await context.waitForEvent("page");
await page.waitForLoadState("domcontentloaded");

await page.addInitScript(() => {
  if (globalThis.__P0_SAR_ACCEPTANCE_RECORDS__) return;
  const originalFetch = globalThis.fetch.bind(globalThis);
  const records = [];
  globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ = records;

  const scalar = (value) => (
    typeof value === "string" || typeof value === "number" || typeof value === "boolean"
      ? value
      : null
  );
  const firstScalar = (value, key, depth = 0) => {
    if (depth > 8 || value == null) return null;
    if (Array.isArray(value)) {
      for (const item of value) {
        const found = firstScalar(item, key, depth + 1);
        if (found !== null) return found;
      }
      return null;
    }
    if (typeof value !== "object") return null;
    if (Object.prototype.hasOwnProperty.call(value, key)) {
      const found = scalar(value[key]);
      if (found !== null) return found;
    }
    for (const child of Object.values(value)) {
      const found = firstScalar(child, key, depth + 1);
      if (found !== null) return found;
    }
    return null;
  };
  const productSummaries = (value) => {
    const products = [];
    const seen = new Set();
    const visit = (candidate, depth = 0) => {
      if (depth > 8 || candidate == null) return;
      if (Array.isArray(candidate)) {
        candidate.forEach((item) => visit(item, depth + 1));
        return;
      }
      if (typeof candidate !== "object") return;
      const productId = firstScalar(candidate, "product_id") ?? firstScalar(candidate, "item_id");
      const dataset = firstScalar(candidate, "dataset");
      if (typeof productId === "string" && typeof dataset === "string" && !seen.has(productId)) {
        seen.add(productId);
        products.push({
          product_id: productId.slice(0, 180),
          dataset: dataset.slice(0, 80),
          processing_level: firstScalar(candidate, "processing_level"),
          product_family: firstScalar(candidate, "product_family"),
          relative_orbit: firstScalar(candidate, "relative_orbit"),
          path: firstScalar(candidate, "path"),
          frame: firstScalar(candidate, "frame"),
          asf_frame: firstScalar(candidate, "asf_frame"),
          acquisition_start: firstScalar(candidate, "acquisition_start")
            ?? firstScalar(candidate, "acquisition_datetime"),
        });
      }
      Object.values(candidate).forEach((child) => visit(child, depth + 1));
    };
    visit(value);
    return products.slice(0, 24);
  };
  const querySummary = (args) => {
    const request = args?.request;
    const query = request?.search?.query ?? {};
    return {
      request_id: typeof request?.request_id === "string" ? request.request_id : null,
      batch_id: typeof request?.batch_id === "string" ? request.batch_id : null,
      route: "legacy_asf_search",
      dataset: typeof query.dataset === "string" ? query.dataset : null,
      processing_levels: Array.isArray(query.processing_levels)
        ? query.processing_levels.slice(0, 16)
        : [],
      aoi_state: query.aoi_geojson ? "geojson" : query.bbox ? "bbox" : query.aoi_wkt ? "wkt" : "none",
      aoi_geometry_type: typeof query.aoi_geojson?.type === "string" ? query.aoi_geojson.type : null,
      date_range: { start: query.start ?? null, end: query.end ?? null },
      filter_state: {
        platforms: Array.isArray(query.platforms) ? query.platforms.slice(0, 16) : [],
        beam_modes: Array.isArray(query.beam_modes) ? query.beam_modes.slice(0, 16) : [],
        polarizations: Array.isArray(query.polarizations) ? query.polarizations.slice(0, 16) : [],
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
    const command = [
      "application_sar_search_and_freeze",
      "application_workbench_page",
      "application_set_working_aoi",
    ].find((candidate) => url.includes(candidate)) ?? null;
    const tracked = command !== null;
    let args = {};
    if (tracked) {
      try {
        args = JSON.parse(await bodyText(input, init));
      } catch {
        args = {};
      }
    }
    const record = tracked
      ? {
          command,
          started_at_unix_ms: Date.now(),
          request: command === "application_sar_search_and_freeze"
            ? querySummary(args)
            : command === "application_workbench_page"
              ? {
                  batch_id: typeof (args?.request?.request ?? args?.request)?.batch_id === "string"
                    ? (args.request.request ?? args.request).batch_id
                    : null,
                  page_index: (args?.request?.request ?? args?.request)?.page_index ?? null,
                  page_size: (args?.request?.request ?? args?.request)?.page_size ?? null,
                }
            : command === "application_set_working_aoi"
              ? {
                  route: "working_aoi",
                  aoi_state: args?.request?.geometry ? "geojson" : "none",
                  aoi_geometry_type: typeof args?.request?.geometry?.type === "string"
                    ? args.request.geometry.type
                    : null,
                  display_name: typeof args?.request?.display_name === "string"
                    ? args.request.display_name.slice(0, 120)
                    : null,
                }
              : null,
          outcome: "pending",
        }
      : null;
    if (record) records.push(record);
    try {
      const response = await originalFetch(input, init);
      if (record) {
        record.finished_at_unix_ms = Date.now();
        record.outcome = response.headers.get("Tauri-Response") === "ok" ? "ok" : "error";
        let result = null;
        try {
          result = JSON.parse(await response.clone().text());
        } catch {
          result = null;
        }
        if (record.outcome === "ok") {
          record.products = command === "application_workbench_page" ? productSummaries(result) : [];
        } else {
          record.error = {
            code: typeof result?.code === "string" ? result.code : null,
            message_key: typeof result?.message_key === "string" ? result.message_key : null,
            context: typeof result?.context === "string" ? result.context : null,
            diagnostic: result?.diagnostic ?? null,
          };
        }
      }
      return response;
    } catch (error) {
      if (record) {
        record.outcome = "error";
        record.finished_at_unix_ms = Date.now();
        record.error = {
          code: typeof error?.code === "string" ? error.code : null,
          message_key: typeof error?.message_key === "string" ? error.message_key : null,
          context: typeof error?.context === "string" ? error.context : null,
          diagnostic: error?.diagnostic ?? null,
        };
      }
      throw error;
    }
  };
});
await page.reload({ waitUntil: "domcontentloaded" });
await page.getByTestId("sar-max-results").waitFor({ state: "visible", timeout: 60_000 });
const uiReadyAt = Date.now();

const skipTour = page.getByRole("button", { name: "跳过" });
if (await skipTour.count()) await skipTour.click();

const snapshotInputs = async (label) => page.evaluate((snapshotLabel) => {
  const value = (testId) => document.querySelector(`[data-testid="${testId}"]`)?.value ?? null;
  return {
    label: snapshotLabel,
    active_testid: document.activeElement?.getAttribute("data-testid") ?? null,
    path_start: value("sar-path-start"),
    path_end: value("sar-path-end"),
    frame_start: value("sar-frame-start"),
    frame_end: value("sar-frame-end"),
    max_results: value("sar-max-results"),
  };
}, label);

const waitForSarRecord = async (previousCount) => {
  await page.waitForFunction(
    (count) => {
      const records = globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? [];
      return records.filter((record) => record.command === "application_sar_search_and_freeze").length > count
        && records.filter((record) => record.command === "application_sar_search_and_freeze").at(-1)?.outcome !== "pending";
    },
    previousCount,
    { timeout: 120_000 },
  );
  await page.waitForTimeout(1_000);
  return page.evaluate(() => structuredClone(globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? []));
};

const searchButton = page.getByRole("button", { name: "检索并导入" });
const runSearch = async () => {
  const before = await page.evaluate(() => ({
    total: (globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? []).length,
    sar: (globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? [])
      .filter((record) => record.command === "application_sar_search_and_freeze").length,
  }));
  await searchButton.click();
  await waitForSarRecord(before.sar);
  await searchButton.waitFor({ state: "visible", timeout: 120_000 });
  await page.waitForFunction(() => {
    const button = [...document.querySelectorAll("button")]
      .find((candidate) => candidate.textContent?.includes("检索并导入"));
    return Boolean(button) && !button.disabled;
  }, undefined, { timeout: 120_000 });
  await page.waitForTimeout(750);
  const records = await page.evaluate(
    (start) => structuredClone((globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? []).slice(start)),
    before.total,
  );
  const sar = records.find((record) => record.command === "application_sar_search_and_freeze");
  if (!sar || sar.outcome !== "ok") {
    throw new Error(`packaged SAR search failed: ${JSON.stringify(sar?.error ?? null)}`);
  }
  return records.filter((record) => (
    record.command !== "application_workbench_page"
    || record.request?.batch_id === sar.request?.batch_id
  ));
};

const pathStart = page.getByTestId("sar-path-start");
const pathEnd = page.getByTestId("sar-path-end");
const imageCount = page.getByTestId("sar-max-results");
const interactionEvidence = [];

await pathStart.click();
await page.keyboard.type("118");
interactionEvidence.push(await snapshotInputs("path_typed_118"));
await page.keyboard.press("Backspace");
interactionEvidence.push(await snapshotInputs("path_backspace_11"));
await page.keyboard.press("Control+A");
await page.keyboard.type("118");
await pathEnd.click();
await page.keyboard.type("118");
await imageCount.click();
await page.keyboard.press("Control+A");
await page.keyboard.type("20");
interactionEvidence.push(await snapshotInputs("path_replaced_and_max_20"));
await page.keyboard.press("Backspace");
interactionEvidence.push(await snapshotInputs("max_backspace_2"));
await page.keyboard.press("Control+A");
await page.keyboard.type("20");

let records = await runSearch();
const firstSearchRecords = structuredClone(records);
const firstProducts = records
  .filter((record) => record.command === "application_workbench_page")
  .flatMap((record) => record.products ?? []);
if (!firstProducts.some((product) => (
  product.dataset === "SENTINEL-1" && product.processing_level === "SLC"
))) {
  throw new Error("Sentinel-1 Path 118 search returned no SLC product");
}
const validFrameProduct = firstProducts.find((product) => Number.isInteger(product.frame));
if (!validFrameProduct) throw new Error("Sentinel-1 Path 118 search returned no integer Frame sample");
const validFrame = String(validFrameProduct.frame);

const frameStart = page.getByTestId("sar-frame-start");
const frameEnd = page.getByTestId("sar-frame-end");
await frameStart.click();
await page.keyboard.type(validFrame);
await page.keyboard.press("Control+A");
await page.keyboard.type(String(Number(validFrame) + 1));
interactionEvidence.push(await snapshotInputs("frame_select_all_replacement"));
await page.keyboard.press("Control+A");
await page.keyboard.type(validFrame);
await frameEnd.click();
await page.keyboard.type(validFrame);
interactionEvidence.push(await snapshotInputs("valid_frame_restored"));
records = await runSearch();
const frameFilteredRecords = structuredClone(records);
const frameFilteredProducts = records
  .filter((record) => record.command === "application_workbench_page")
  .flatMap((record) => record.products ?? []);
if (!frameFilteredProducts.some((product) => String(product.frame) === validFrame)) {
  throw new Error(`response-derived Frame ${validFrame} did not return the same Frame`);
}

const datasetSelect = page.locator('[data-tour="asf-filter"] select').first();
await datasetSelect.selectOption("ALOS PALSAR");
await page.getByTestId("sar-max-results").fill("20");
records = await runSearch();
const additionalDatasetRecords = structuredClone(records);
const additionalDatasetProducts = records
  .filter((record) => record.command === "application_workbench_page")
  .flatMap((record) => record.products ?? []);
if (!additionalDatasetProducts.some((product) => product.dataset === "ALOS PALSAR")) {
  throw new Error("ALOS PALSAR generic ASF search returned no product");
}

await datasetSelect.selectOption("SENTINEL-1");
await page.getByRole("button", { name: "更换范围" }).click();
await page.getByRole("button", { name: /^经纬度范围/ }).click();
await page.getByPlaceholder("West").fill("110");
await page.getByPlaceholder("East").fill("112");
await page.getByPlaceholder("South").fill("30");
await page.getByPlaceholder("North").fill("32");
const workingAoiCount = await page.evaluate(() => (
  globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? []
).filter((record) => record.command === "application_set_working_aoi").length);
await page.getByRole("button", { name: "绑定矩形 AOI" }).click();
await page.waitForFunction(
  (count) => {
    const records = (globalThis.__P0_SAR_ACCEPTANCE_RECORDS__ ?? [])
      .filter((record) => record.command === "application_set_working_aoi");
    return records.length > count && records.at(-1)?.outcome === "ok";
  },
  workingAoiCount,
  { timeout: 30_000 },
);
records = await runSearch();
const aoiSearchRecords = structuredClone(records);
const aoiProducts = records
  .filter((record) => record.command === "application_workbench_page")
  .flatMap((record) => record.products ?? []);
if (!aoiProducts.some((product) => (
  product.dataset === "SENTINEL-1" && product.processing_level === "SLC"
))) {
  throw new Error("working-AOI Sentinel-1 search returned no SLC product");
}

await page.getByTestId("sar-path-start").click();
await page.keyboard.press("Control+A");
await page.keyboard.press("Backspace");
await page.getByTestId("sar-path-end").click();
await page.keyboard.press("Control+A");
await page.keyboard.press("Backspace");
await page.getByTestId("sar-frame-start").click();
await page.keyboard.press("Control+A");
await page.keyboard.press("Backspace");
await page.getByTestId("sar-frame-end").click();
await page.keyboard.press("Control+A");
await page.keyboard.press("Backspace");
interactionEvidence.push(await snapshotInputs("optional_ranges_empty"));

const sarRecords = (items) => items.filter((record) => record.command === "application_sar_search_and_freeze");
const pageProducts = (items) => items
  .filter((record) => record.command === "application_workbench_page")
  .flatMap((record) => record.products ?? []);
const finalEvidence = {
  schema_version: 1,
  evidence_kind: "real_packaged_tauri_webview_public_asf_no_download",
  captured_at_utc: new Date().toISOString(),
  cdp_endpoint: endpoint,
  ui_ready: {
    observed_unix_ms: uiReadyAt,
    process_launch_to_ui_ready_ms: processStartedAt > 0 ? uiReadyAt - processStartedAt : null,
  },
  interaction: interactionEvidence,
  sentinel1_no_aoi: {
    requests: sarRecords(firstSearchRecords).slice(-1),
    products: pageProducts(firstSearchRecords).slice(-24),
  },
  sentinel1_valid_frame: {
    response_derived_frame: validFrame,
    source_product_id: validFrameProduct.product_id,
    requests: sarRecords(frameFilteredRecords).slice(-1),
    products: pageProducts(frameFilteredRecords).slice(-24),
  },
  additional_asf_dataset_no_aoi: {
    dataset: "ALOS PALSAR",
    requests: sarRecords(additionalDatasetRecords).slice(-1),
    products: pageProducts(additionalDatasetRecords).slice(-24),
  },
  sentinel1_working_aoi: {
    request_geometry_redacted: true,
    requests: sarRecords(aoiSearchRecords).slice(-1),
    products: pageProducts(aoiSearchRecords).slice(-24),
  },
  downloads_started: false,
};

await mkdir(path.dirname(evidencePath), { recursive: true });
await writeFile(evidencePath, `${JSON.stringify(finalEvidence, null, 2)}\n`, "utf8");
if (screenshotPath) {
  await mkdir(path.dirname(screenshotPath), { recursive: true });
  await page.screenshot({ path: screenshotPath, fullPage: false });
}
await browser.close();
