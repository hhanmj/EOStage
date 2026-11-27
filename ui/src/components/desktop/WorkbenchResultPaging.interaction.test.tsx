import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
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
import type { SourceDescriptor, SourceView } from "@/shell/api/types";

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

const resourceId = "science.sentinel-2.l2a.v1";
const batchId = "sentinel-2-288";
const revision = 7;

function storeItem(index: number) {
  const itemId = `s2-item-${String(index + 1).padStart(3, "0")}`;
  return {
    item_id: itemId,
    provider_id: "copernicus.dataspace.odata",
    display_name: `S2A_MSIL2A_20261004T032601_N0513_R018_T48RXQ_20261004T09${String(index).padStart(4, "0")}.SAFE`,
    acquisition_datetime: "2026-10-04T03:26:01Z",
    footprint_geojson: null,
    metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      product_kind: "optical_scene",
      properties: { cloud_cover: index % 100, product_type: "S2MSI2A" },
      assets: [{ asset_key: "archive", size_bytes: 733061241 }],
    },
    payload: {},
    asset_access: { expected_size_bytes: 733061241 },
    execution_policy: null,
  };
}

const items = Array.from({ length: 288 }, (_, index) => storeItem(index));

function installBridge() {
  const calls: Array<{ page: number; pageSize: number }> = [];
  const base = createWorkbenchTestDesktopShellBridge();
  const source: SourceDescriptor = {
    resource_id: resourceId,
    group_id: "sentinel-2",
    display_name_zh: "Sentinel-2 L2A",
    family: "optical",
    search_mode: "catalogue_intent",
    filter_schema_id: "search.optical.sentinel-2.l2a.v1",
    filter_schema_revision: 1,
    result_cap: 300,
    accounts: ["cdse"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  };
  const sarSource: SourceDescriptor = {
    ...source,
    resource_id: "science.sentinel-1.standard.v1",
    group_id: "sentinel-1",
    display_name_zh: "Sentinel-1",
    family: "sar",
    search_mode: "sar",
    filter_schema_id: null,
    filter_schema_revision: null,
    accounts: ["earthdata"],
  };
  const sourceView: SourceView = {
    resource_id: resourceId,
    batch_id: batchId,
    revision,
    item_count: items.length,
    searched_at: "2026-10-31T11:09:00Z",
    pending_request_id: null,
    last_error: null,
  };
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      unified: {
        ...base.application.unified,
        listSources: async () => [sarSource, source],
        sourceViews: async () => [sourceView],
        batchPage: async (requestedBatchId, requestedRevision, page, pageSize) => {
          calls.push({ page, pageSize });
          if (requestedBatchId === "sentinel-1-initial") {
            return { batch_id: requestedBatchId, revision: requestedRevision, page, page_size: pageSize, total_count: 0, items: [], rows: [] };
          }
          assert.equal(requestedBatchId, batchId);
          assert.equal(requestedRevision, revision);
          return {
            batch_id: batchId,
            revision,
            page,
            page_size: pageSize,
            total_count: items.length,
            items: items.slice(page * pageSize, (page + 1) * pageSize),
            rows: [],
          };
        },
      },
    },
  });
  return calls;
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.clear();
  window.location.hash = "";
});

test("load more reaches the second batch page (250/page) and select-all covers every loaded row", async () => {
  const calls = installBridge();
  window.location.hash = "#/workbench";
  const view = render(<DesktopWorkbench dark={false} onToggleDark={() => {}} />);

  await waitFor(() => assert.ok(view.getByTestId("manual-workbench")));
  const rail = view.container.querySelector("nav") as HTMLElement;
  assert.equal(rail.dataset.sidebarState, "collapsed", "entering map workbench auto-collapses the left sidebar");

  const sourceSelect = await waitFor(() => view.getByTestId("manual-workbench").querySelector(`select:has(option[value="${resourceId}"])`) as HTMLSelectElement);
  fireEvent.change(sourceSelect, { target: { value: resourceId } });
  await waitFor(() => assert.ok(view.getByText("288 景", { exact: true })));
  await waitFor(() => assert.ok(calls.some((call) => call.page === 0 && call.pageSize === 250)));
  assert.ok(view.container.querySelector('[data-scene-id="s2-item-001"]'));
  assert.ok(view.container.querySelector('[data-scene-id="s2-item-250"]'));
  assert.equal(view.container.querySelector('[data-scene-id="s2-item-251"]'), null);
  assert.match(view.container.querySelector('[data-scene-id="s2-item-001"]')?.textContent ?? "", /Sentinel-2A/);
  assert.match(view.container.querySelector('[data-scene-id="s2-item-001"]')?.textContent ?? "", /Tile T48RXQ/);
  assert.doesNotMatch(view.container.querySelector('[data-scene-id="s2-item-001"]')?.textContent ?? "", /Path --|未知/);

  fireEvent.click(view.getByRole("button", { name: "全选已加载", exact: true }));
  assert.ok(view.getByText(/^已勾选 250/));

  fireEvent.click(view.getByRole("button", { name: "加载更多", exact: true }));
  await waitFor(() => assert.ok(calls.some((call) => call.page === 1 && call.pageSize === 250)));
  await waitFor(() => assert.equal(view.queryByRole("button", { name: "加载更多", exact: true }), null));

  fireEvent.click(view.getByRole("button", { name: "全选已加载", exact: true }));
  assert.ok(view.getByText(/^已勾选 288/));
  fireEvent.click(view.getByRole("button", { name: "下一页", exact: true }));
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s2-item-251"]')));
  assert.ok(view.getByText("第 2 / 2 页", { exact: true }));
});

test("left sidebar auto-collapses on map workbench and restores when returning to chat", async () => {
  installBridge();
  window.location.hash = "#/agent";
  const view = render(<DesktopWorkbench dark={false} onToggleDark={() => {}} />);

  const rail = () => view.container.querySelector("nav") as HTMLElement;
  await waitFor(() => assert.equal(rail().dataset.sidebarState, "expanded"));

  fireEvent.click(view.getByRole("button", { name: "地图工作台" }));
  await waitFor(() => assert.equal(rail().dataset.sidebarState, "collapsed"));

  fireEvent.click(view.getByRole("button", { name: "新对话" }));
  await waitFor(() => assert.equal(rail().dataset.sidebarState, "expanded"));
});
