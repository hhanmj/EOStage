import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { within } from "@testing-library/react";
import { waitFor } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge";
import type { CreateTaskOptions, SourceDescriptor, SourceView } from "@/shell/api/types";
import type { SceneRow } from "@/lib/bridge";

const scopedSource = (url: string) => `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopWorkbench.tsx") || url.endsWith("/DesktopAgentWorkbench.tsx")) {
      return {
        format: "module",
        shortCircuit: true,
        source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code,
      };
    }
    return nextLoad(url, context);
  },
});
const { DesktopWorkbench } = await import("./DesktopWorkbench");
hooks.deregister();

const SAR_RESOURCE_ID = "science.sentinel-1.standard.v1";
const OPTICAL_RESOURCE_ID = "science.sentinel-2.l2a.v1";
const DEFAULT_OUTPUT = "D:\\ScenePort\\default";
const CHOSEN_OUTPUT = "D:\\ScenePort\\chosen";

type TaskCall = {
  batchId: string;
  revision: number;
  itemIds: string[];
  options: CreateTaskOptions;
};

function source(resourceId: string, family: "sar" | "optical"): SourceDescriptor {
  return {
    resource_id: resourceId,
    group_id: family === "sar" ? "sentinel-1" : "sentinel-2",
    display_name_zh: family === "sar" ? "Sentinel-1" : "Sentinel-2 L2A",
    family,
    search_mode: family === "sar" ? "sar" : "catalogue_intent",
    filter_schema_id: family === "sar" ? null : "search.optical.sentinel-2.l2a.v1",
    filter_schema_revision: family === "sar" ? null : 1,
    result_cap: 100,
    accounts: family === "sar" ? ["earthdata"] : ["cdse"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  };
}

function view(resourceId: string, batchId: string): SourceView {
  return {
    resource_id: resourceId,
    batch_id: batchId,
    revision: 7,
    item_count: 2,
    searched_at: "2026-11-01T09:00:00Z",
    pending_request_id: null,
    last_error: null,
  };
}

function scene(resourceId: string, index: number): SceneRow {
  const optical = resourceId === OPTICAL_RESOURCE_ID;
  return {
    scene_id: `${optical ? "s2" : "s1"}-item-${index}`,
    platform: optical ? "Sentinel-2A" : "Sentinel-1A",
    product_type: optical ? "S2MSI2A" : "SLC",
    beam_mode: optical ? "" : "IW",
    polarization: optical ? "" : "VV+VH",
    acquisition_datetime: "2026-10-31T03:26:01Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: optical ? "" : "ascending",
    has_url: false,
    native_metadata: {
      logical_dataset: optical ? "optical.sentinel-2.l2a" : "sentinel-1.slc",
      product_kind: optical ? "optical_scene" : "sar_scene",
    },
  };
}

function storeItem(resourceId: string, index: number) {
  const row = scene(resourceId, index);
  const optical = resourceId === OPTICAL_RESOURCE_ID;
  return {
    item_id: row.scene_id,
    provider_id: optical ? "copernicus.dataspace.odata" : "asf",
    display_name: row.scene_id,
    acquisition_datetime: row.acquisition_datetime,
    footprint_geojson: null,
    metadata: {
      logical_dataset: optical ? "optical.sentinel-2.l2a" : "sentinel-1.slc",
      product_kind: optical ? "optical_scene" : "sar_scene",
      product_type: row.product_type,
      platform: row.platform,
      beam_mode: row.beam_mode,
      polarization: row.polarization,
      orbit_direction: row.orbit_direction,
      properties: optical ? { product_type: row.product_type } : {},
    },
    payload: {},
    asset_access: { expected_size_bytes: 1024 },
    execution_policy: null,
  };
}

function installBridge(resourceId: string, family: "sar" | "optical") {
  const taskCalls: TaskCall[] = [];
  const batchReads: string[] = [];
  let batchNumber = 1;
  let views = [view(resourceId, "batch-1")];
  const base = createWorkbenchTestDesktopShellBridge();
  const currentSource = source(resourceId, family);
  const sources = family === "sar"
    ? [currentSource, source(OPTICAL_RESOURCE_ID, "optical")]
    : [source(SAR_RESOURCE_ID, "sar"), currentSource];

  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      getWorkingAoi: async () => ({
        source_kind: "drawn",
        geometry: { type: "Polygon", coordinates: [] },
        bbox: { west: 120, south: 30, east: 121, north: 31 },
        crs: "EPSG:4326",
        display_name: "Test AOI",
      }),
      getSarDownloadPolicySettings: async () => ({
        mode: "manual",
        max_concurrent_items: 3,
        organize_by_product: true,
        default_output_root: DEFAULT_OUTPUT,
      }),
      unified: {
        ...base.application.unified,
        listSources: async () => sources,
        sourceViews: async () => views,
        searchStart: async (request) => {
          batchNumber += 1;
          views = [view(request.resource_id, `batch-${batchNumber}`)];
          return {
            request_id: "search-next",
            resource_id: request.resource_id,
            stage: "requesting",
            requests_done: 0,
            items_kept: 0,
            items_cap: 100,
            elapsed_ms: 0,
            message_zh: "检索中",
            terminal: false,
            error: null,
          };
        },
        searchProgress: async (requestId) => ({
          request_id: requestId,
          resource_id: resourceId,
          stage: "done",
          requests_done: 1,
          items_kept: 2,
          items_cap: 100,
          elapsed_ms: 250,
          message_zh: "检索完成",
          terminal: true,
          error: null,
        }),
        batchPage: async (batchId, revision, page, pageSize) => {
          batchReads.push(batchId);
          return {
            batch_id: batchId,
            revision,
            page,
            page_size: pageSize,
            total_count: 2,
            items: [storeItem(resourceId, 1), storeItem(resourceId, 2)],
            rows: [],
          };
        },
        createTask: async (batchId, revision, itemIds, options) => {
          taskCalls.push({ batchId, revision, itemIds, options });
          return { task_id: `task-${taskCalls.length}`, revision: 1, status: "queued", created: true };
        },
      },
      ...(family === "optical"
        ? {
            cdseSessionStatus: async () => ({
              ok: true as const,
              status: {
                network_mode: "direct" as const,
                authenticated: true,
                refresh_available: true,
                expires_in_seconds: 600,
              },
            }),
          }
        : {}),
    },
    dialog: {
      ...base.dialog,
      pickDirectory: async () => ({ ok: true, path: CHOSEN_OUTPUT }),
    },
  });
  return { taskCalls, batchReads };
}

async function renderWorkbench(resourceId = SAR_RESOURCE_ID) {
  window.location.hash = "#/workbench";
  const rendered = render(<DesktopWorkbench dark={false} onToggleDark={() => {}} />);
  await waitFor(() => assert.ok(rendered.getByTestId("manual-workbench")));
  if (resourceId !== SAR_RESOURCE_ID) {
    const select = await waitFor(() => rendered.getByTestId("manual-workbench").querySelector(
      `select:has(option[value="${resourceId}"])`,
    ) as HTMLSelectElement);
    fireEvent.change(select, { target: { value: resourceId } });
  }
  await waitFor(() => assert.ok(rendered.container.querySelector(`[data-scene-id="${resourceId === SAR_RESOURCE_ID ? "s1" : "s2"}-item-1"]`)));
  return rendered;
}

function rowDownload(view: ReturnType<typeof render>, index: number, family: "sar" | "optical" = "sar") {
  const row = view.container.querySelector(`[data-scene-id="${family === "sar" ? "s1" : "s2"}-item-${index}"]`);
  assert.ok(row, `missing result row ${index}`);
  return within(row as HTMLElement).getByRole("button", { name: "下载", exact: true });
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.clear();
  window.location.hash = "";
});

test("first SAR row download confirms directory and task options, later rows go straight, and a new search requires confirmation again", async () => {
  const { taskCalls, batchReads } = installBridge(SAR_RESOURCE_ID, "sar");
  const rendered = await renderWorkbench();

  fireEvent.click(rowDownload(rendered, 1));
  const dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(taskCalls.length, 0);
  assert.equal((within(dialog).getByLabelText("目的目录") as HTMLInputElement).value, DEFAULT_OUTPUT);
  const orbitCheckbox = within(dialog).getByRole("checkbox", { name: "同时下载精密轨道（Orbit / POEORB）" }) as HTMLInputElement;
  assert.equal(orbitCheckbox.checked, true);
  assert.ok(within(dialog).getByText("已默认附带精密轨道"));

  fireEvent.click(within(dialog).getByRole("button", { name: "选择目录" }));
  await waitFor(() => assert.equal((within(dialog).getByLabelText("目的目录") as HTMLInputElement).value, CHOSEN_OUTPUT));
  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(taskCalls.length, 1));
  await waitFor(() => assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null));
  assert.deepEqual(taskCalls[0], {
    batchId: "batch-1",
    revision: 7,
    itemIds: ["s1-item-1"],
    options: { output_root: CHOSEN_OUTPUT, concurrency: 3, layout: "sar_dataset_processing", include_precise_orbit: true },
  });

  fireEvent.click(rowDownload(rendered, 2));
  await waitFor(() => assert.equal(taskCalls.length, 2));
  assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null);
  assert.equal(taskCalls[1].options.output_root, CHOSEN_OUTPUT);
  assert.equal(taskCalls[1].options.include_precise_orbit, true);
  assert.deepEqual(taskCalls[1].itemIds, ["s1-item-2"]);

  fireEvent.click(rendered.getByRole("button", { name: "检索", exact: true }));
  await waitFor(() => assert.ok(batchReads.includes("batch-2")));
  fireEvent.click(rowDownload(rendered, 1));
  assert.ok(await rendered.findByRole("dialog", { name: "确认下载任务" }));
  assert.equal(taskCalls.length, 2);
});

test("cancelling the row confirmation does not confirm the batch", async () => {
  const { taskCalls } = installBridge(SAR_RESOURCE_ID, "sar");
  const rendered = await renderWorkbench();

  fireEvent.click(rowDownload(rendered, 1));
  let dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  fireEvent.click(within(dialog).getByRole("button", { name: "取消", exact: true }));
  await waitFor(() => assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null));
  assert.equal(taskCalls.length, 0);

  fireEvent.click(rowDownload(rendered, 1));
  dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  assert.ok(dialog);
  assert.equal(taskCalls.length, 0);
});

test("checked downloads always confirm, create one task for all checked ids, and cancel creates nothing", async () => {
  const { taskCalls } = installBridge(SAR_RESOURCE_ID, "sar");
  const rendered = await renderWorkbench();

  fireEvent.click(within(rendered.container.querySelector('[data-scene-id="s1-item-1"]') as HTMLElement)
    .getByRole("button", { name: "勾选", exact: true }));
  fireEvent.click(within(rendered.container.querySelector('[data-scene-id="s1-item-2"]') as HTMLElement)
    .getByRole("button", { name: "勾选", exact: true }));
  fireEvent.click(rendered.getByRole("button", { name: "加入下载队列", exact: true }));
  let dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(taskCalls.length, 0);
  assert.equal(within(dialog).getByText("内容：", { exact: true }).parentElement?.textContent, "内容：2 个产品");

  const checkedOrbitCheckbox = within(dialog).getByRole("checkbox", { name: "同时下载精密轨道（Orbit / POEORB）" }) as HTMLInputElement;
  assert.equal(checkedOrbitCheckbox.checked, true);
  assert.ok(within(dialog).getByText("已默认附带精密轨道"));

  fireEvent.click(within(dialog).getByRole("button", { name: "选择目录" }));
  await waitFor(() => assert.equal((within(dialog).getByLabelText("目的目录") as HTMLInputElement).value, CHOSEN_OUTPUT));
  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(taskCalls.length, 1));
  assert.deepEqual(taskCalls[0], {
    batchId: "batch-1",
    revision: 7,
    itemIds: ["s1-item-1", "s1-item-2"],
    options: { output_root: CHOSEN_OUTPUT, concurrency: 3, layout: "sar_dataset_processing", include_precise_orbit: true },
  });
  await waitFor(() => assert.equal(
    (rendered.getByRole("button", { name: "加入下载队列", exact: true }) as HTMLButtonElement).disabled,
    true,
  ));

  // The checked confirmation also supplies this batch's carried-over row settings.
  fireEvent.click(rowDownload(rendered, 1));
  await waitFor(() => assert.equal(taskCalls.length, 2));
  assert.equal(taskCalls[1].options.output_root, CHOSEN_OUTPUT);
  assert.equal(taskCalls[1].options.include_precise_orbit, true);
  assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null);

  fireEvent.click(within(rendered.container.querySelector('[data-scene-id="s1-item-2"]') as HTMLElement)
    .getByRole("button", { name: "勾选", exact: true }));
  fireEvent.click(rendered.getByRole("button", { name: "加入下载队列", exact: true }));
  dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(taskCalls.length, 2);
  fireEvent.click(within(dialog).getByRole("button", { name: "取消", exact: true }));
  await waitFor(() => assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null));
  assert.equal(taskCalls.length, 2);

  fireEvent.click(rendered.getByRole("button", { name: "加入下载队列", exact: true }));
  assert.ok(await rendered.findByRole("dialog", { name: "确认下载任务" }));
  assert.equal(taskCalls.length, 2);
});

test("an optical row uses the same first-download confirmation (exposing delivery/band options) and then goes straight", async () => {
  const { taskCalls } = installBridge(OPTICAL_RESOURCE_ID, "optical");
  const rendered = await renderWorkbench(OPTICAL_RESOURCE_ID);

  fireEvent.click(rowDownload(rendered, 1, "optical"));
  const dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(taskCalls.length, 0);
  // Item 6: Sentinel-2 L2A confirmation exposes product label & explicit SAFE / analysis-band choice.
  assert.ok(within(dialog).getByText("Sentinel-2 L2A", { exact: true }));
  assert.ok(within(dialog).getByRole("radio", { name: /完整 SAFE 产品/ }));
  assert.ok(within(dialog).getByRole("radio", { name: /分析波段/ }));
  assert.equal(within(dialog).queryByText("generic_sar", { exact: true }), null);
  assert.equal(within(dialog).queryByRole("checkbox", { name: "同时下载精密轨道（Orbit / POEORB）" }), null);
  assert.ok(within(dialog).queryByText("已默认附带精密轨道") === null);
  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(taskCalls.length, 1));
  assert.deepEqual(taskCalls[0].itemIds, ["s2-item-1"]);

  fireEvent.click(rowDownload(rendered, 2, "optical"));
  await waitFor(() => assert.equal(taskCalls.length, 2));
  assert.equal(rendered.queryByRole("dialog", { name: "确认下载任务" }), null);
  assert.deepEqual(taskCalls[1].itemIds, ["s2-item-2"]);
});

test("unchecking precise orbit in dialog cancels companion orbit and persists preference", async () => {
  const { taskCalls, batchReads } = installBridge(SAR_RESOURCE_ID, "sar");
  const rendered = await renderWorkbench();

  fireEvent.click(rowDownload(rendered, 1));
  const dialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  const orbitCheckbox = within(dialog).getByRole("checkbox", { name: "同时下载精密轨道（Orbit / POEORB）" }) as HTMLInputElement;
  assert.equal(orbitCheckbox.checked, true);
  assert.ok(within(dialog).getByText("已默认附带精密轨道"));

  // Uncheck the precise orbit checkbox
  fireEvent.click(orbitCheckbox);
  assert.equal(orbitCheckbox.checked, false);
  assert.ok(within(dialog).queryByText("已默认附带精密轨道") === null);

  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(taskCalls.length, 1));
  assert.equal(taskCalls[0].options.include_precise_orbit, false);

  // Subsequent download in same batch also gets false
  fireEvent.click(rowDownload(rendered, 2));
  await waitFor(() => assert.equal(taskCalls.length, 2));
  assert.equal(taskCalls[1].options.include_precise_orbit, false);

  // In a new search, the remembered preference is false (uncheck persisted)
  fireEvent.click(rendered.getByRole("button", { name: "检索", exact: true }));
  await waitFor(() => assert.ok(batchReads.includes("batch-2")));
  fireEvent.click(rowDownload(rendered, 1));
  const newDialog = await rendered.findByRole("dialog", { name: "确认下载任务" });
  const newCheckbox = within(newDialog).getByRole("checkbox", { name: "同时下载精密轨道（Orbit / POEORB）" }) as HTMLInputElement;
  assert.equal(newCheckbox.checked, false);
  assert.ok(within(newDialog).queryByText("已默认附带精密轨道") === null);
});
