import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { waitFor, within } from "@testing-library/react";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";
import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "@/lib/desktopShellBridge";
import { createWorkbenchTestDesktopShellBridge } from "../../test-utils/workbenchDesktopBridge";
import { I18nProvider, LOCALE_STORAGE_KEY } from "../../i18n";
import type {
  BatchAllMatchCreateTaskRequest,
  BatchAllMatchPreviewRequest,
  CreateTaskOptions,
  SourceDescriptor,
  SourceView,
} from "@/shell/api/types";
import type { SearchBatchProgressState, SearchProgress } from "@/lib/generated/v3-contracts";

// #22 B slice 5: a search shows its first page while the rest keeps loading
// (contract 3.41.0). Arrived pages browse normally, a page that has not
// arrived shows a loading state and fills in, the search can be cancelled
// (rows kept, possibly incomplete), select-all over every match waits for
// the batch to finish, and arrived rows download row by row.

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

const SAR = "science.sentinel-1.standard.v1";
const OUTPUT = "D:\\ScenePort\\default";
const LIVE_BATCH = "batch-live";
const REQUEST = "search-live";

function sarSource(resourceId = SAR, family: "sar" | "optical" = "sar"): SourceDescriptor {
  return {
    resource_id: resourceId,
    group_id: family === "sar" ? "sentinel-1" : "optical",
    display_name_zh: resourceId,
    family,
    search_mode: family === "sar" ? "sar" : "catalogue_intent",
    filter_schema_id: null,
    filter_schema_revision: null,
    result_cap: 10_000,
    accounts: ["earthdata"],
    network_default: "direct",
    availability: "available",
    availability_reason_zh: null,
  };
}

function sourceView(batchId: string, revision: number, count: number, resourceId = SAR): SourceView {
  return {
    resource_id: resourceId,
    batch_id: batchId,
    revision,
    item_count: count,
    searched_at: "2026-11-03T09:00:00Z",
    pending_request_id: null,
    last_error: null,
  };
}

/** Item `index` (1-based); newer first, so the default date sort keeps the frozen order. */
function storeItem(index: number) {
  const id = `s1-item-${String(index).padStart(3, "0")}`;
  const at = new Date(Date.UTC(2026, 9, 31) - index * 60_000).toISOString();
  return {
    item_id: id,
    provider_id: "asf",
    display_name: id,
    acquisition_datetime: at,
    footprint_geojson: null,
    metadata: {
      logical_dataset: "sentinel-1.slc",
      product_kind: "sar_scene",
      product_type: "SLC",
      platform: "Sentinel-1A",
      beam_mode: "IW",
      polarization: "VV+VH",
      orbit_direction: "ascending",
    },
    payload: {},
    asset_access: { expected_size_bytes: 1024 },
    execution_policy: null,
  };
}

type Script = {
  arrived: number;
  remoteTotal: number | null;
  state: SearchBatchProgressState;
  taskCalls: Array<{ batchId: string; revision: number; itemIds: string[]; options: CreateTaskOptions }>;
  allMatchPreviewCalls: BatchAllMatchPreviewRequest[];
  allMatchCreateCalls: BatchAllMatchCreateTaskRequest[];
  cancels: string[];
  searchStartCalls: number;
};

function installBridge(
  arrived: number,
  remoteTotal: number | null,
  shownState: SearchBatchProgressState = "complete",
  resourceId = SAR,
  family: "sar" | "optical" = "sar",
): Script {
  const script: Script = {
    arrived,
    remoteTotal,
    state: "loading",
    taskCalls: [],
    allMatchPreviewCalls: [],
    allMatchCreateCalls: [],
    cancels: [],
    searchStartCalls: 0,
  };
  let views: SourceView[] =
    resourceId === SAR
      ? [sourceView("batch-old", 7, 2)]
      : [sourceView("batch-old", 7, 2), sourceView("batch-old-alos2", 7, 2, resourceId)];
  const batchIdCount = (batchId: string) => (batchId === LIVE_BATCH ? script.arrived : 2);
  const base = createWorkbenchTestDesktopShellBridge();
  const progress = (): SearchProgress => {
    const terminal = script.state !== "loading";
    return {
      request_id: REQUEST,
      resource_id: resourceId,
      stage: script.state === "loading" ? "requesting" : script.state === "complete" ? "done" : "cancelled",
      requests_done: 1,
      items_kept: script.arrived,
      items_cap: 10_000,
      elapsed_ms: 100,
      message_zh: "",
      terminal,
      error: null,
      possibly_incomplete: script.state === "cancelled",
      batch_id: LIVE_BATCH,
      loaded_count: script.arrived,
      remote_total: script.remoteTotal,
    };
  };
  registerTestDesktopShellBridge({
    ...base,
    application: {
      ...base.application,
      getWorkingAoi: async () => ({
        source_kind: "drawn",
        geometry: { type: "Polygon", coordinates: [[[120, 30], [121, 30], [121, 31], [120, 30]]] },
        bbox: { west: 120, south: 30, east: 121, north: 31 },
        crs: "EPSG:4326",
        display_name: "Test AOI",
      }),
      getSarDownloadPolicySettings: async () => ({
        mode: "manual",
        max_concurrent_items: 3,
        organize_by_product: true,
        default_output_root: OUTPUT,
      }),
      unified: {
        ...base.application.unified,
        listSources: async () =>
          resourceId === SAR ? [sarSource()] : [sarSource(), sarSource(resourceId, family)],
        sourceViews: async () =>
          script.state === "complete" && script.searchStartCalls > 0
            ? [sourceView(LIVE_BATCH, 0, script.arrived, resourceId)]
            : views,
        searchStart: async (request) => {
          script.searchStartCalls += 1;
          return {
            request_id: REQUEST,
            resource_id: request.resource_id,
            stage: "requesting",
            requests_done: 0,
            items_kept: 0,
            items_cap: 10_000,
            elapsed_ms: 0,
            message_zh: "",
            terminal: false,
            error: null,
          };
        },
        searchProgress: async () => progress(),
        cancelCatalogSearch: async (requestId) => {
          script.cancels.push(requestId);
          script.state = "cancelled";
          views = [sourceView(LIVE_BATCH, 0, script.arrived, resourceId)];
          return { request_id: requestId, cancelled: true, batch_id: LIVE_BATCH, state: "cancelled" };
        },
        batchPage: async (batchId, revision, page, pageSize) => {
          if (batchId !== LIVE_BATCH) {
            return {
              batch_id: batchId, revision, page, page_size: pageSize, total_count: 2,
              items: [storeItem(901), storeItem(902)], rows: [], state: shownState, loaded_count: 2,
            };
          }
          const start = page * pageSize;
          if (script.state === "loading" && start >= script.arrived) {
            throw { code: "V3_STORE_BATCH_PAGE_NOT_YET_LOADED", message_zh: "not yet" };
          }
          const items = Array.from(
            { length: Math.max(0, Math.min(pageSize, script.arrived - start)) },
            (_, offset) => storeItem(start + offset + 1),
          );
          return {
            batch_id: batchId,
            revision,
            page,
            page_size: pageSize,
            total_count: script.arrived,
            items,
            rows: items.map((item) => {
              const call = script.taskCalls.find((entry) => entry.itemIds.includes(item.item_id));
              return call
                ? { item_id: item.item_id, task_id: "task-live-1", task_status: "queued", item_status: "queued", actions: ["open_task", "pause"] }
                : { item_id: item.item_id, task_id: null, task_status: null, item_status: null, actions: ["download"] };
            }),
            state: script.state,
            loaded_count: script.arrived,
            remote_total: script.remoteTotal,
          };
        },
        createTask: async (batchId, revision, itemIds, options) => {
          script.taskCalls.push({ batchId, revision, itemIds, options });
          return { task_id: "task-live-1", revision: 1, status: "queued", created: true };
        },
        previewAllMatchTask: async (request) => {
          script.allMatchPreviewCalls.push(request);
          const total = batchIdCount(request.batch_id);
          const excluded = request.excluded_item_ids?.length ?? 0;
          const admissible = Math.max(0, total - excluded - 2);
          return {
            batch_id: request.batch_id,
            revision: request.expected_revision,
            batch_total_count: total,
            matched_count: total - excluded,
            excluded_count: excluded,
            skipped_active_count: 1,
            rejected_count: 1,
            admissible_count: admissible,
            known_bytes: admissible * 1024,
            unknown_size_count: 0,
            selection_digest: `digest-${request.batch_id}-${admissible}`,
          };
        },
        createAllMatchTask: async (request) => {
          script.allMatchCreateCalls.push(request);
          return {
            task_id: "task-all-match-1",
            revision: 1,
            status: "queued",
            created: true,
            matched_count: request.expected_admissible_count + 2,
            created_count: request.expected_admissible_count,
            pending_count: request.expected_admissible_count,
            skipped_count: 1,
            rejected_count: 1,
          };
        },
      },
    },
  });
  return script;
}

async function startLoadingSearch(locale: "zh-CN" | "en" = "zh-CN") {
  window.location.hash = "#/workbench";
  window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  const view = render(
    <I18nProvider>
      <DesktopWorkbench dark={false} onToggleDark={() => {}} />
    </I18nProvider>,
  );
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-901"]')));
  fireEvent.click(view.getByRole("button", { name: locale === "en" ? "Search" : "检索", exact: true }));
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-001"]')), { timeout: 3000 });
  return view;
}

function row(view: ReturnType<typeof render>, index: number): HTMLElement {
  const element = view.container.querySelector(`[data-scene-id="s1-item-${String(index).padStart(3, "0")}"]`);
  assert.ok(element, `missing row ${index}`);
  return element as HTMLElement;
}

test.afterEach(() => {
  cleanup();
  clearTestDesktopShellBridge();
  window.localStorage.clear();
  window.location.hash = "";
});

test("the provider total leads, a page that has not arrived shows loading and fills in", async () => {
  const script = installBridge(300, 600);
  const view = await startLoadingSearch();

  assert.ok(view.getByText("600 景", { exact: true }));
  assert.equal(view.getByTestId("result-loaded-count").textContent, "已载入 300 / 600");
  assert.ok(view.getByText("第 1 / 3 页", { exact: true }));

  fireEvent.click(view.getByRole("button", { name: "下一页", exact: true }));
  fireEvent.click(view.getByRole("button", { name: "下一页", exact: true }));
  await waitFor(() => assert.ok(view.getByText("第 3 / 3 页", { exact: true })));
  await waitFor(() => assert.ok(view.getByTestId("result-page-loading")));
  assert.equal(view.container.querySelector("[data-scene-id]"), null, "a page that has not arrived is never an empty list");

  script.arrived = 600;
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-501"]')), { timeout: 4000 });
  assert.ok(view.container.querySelector('[data-scene-id="s1-item-600"]'));
  await waitFor(() => assert.equal(view.getByTestId("result-loaded-count").textContent, "已载入 600 / 600"));
});

test("select-all over every match hides below 250 scenes and waits when >= 250 while rows can still be checked and downloaded one by one", async () => {
  // Owner 2026-11-16: 「全选全部匹配的 xx 景」统一精简为「全选全部」，且小于 250 景时不需要出现
  installBridge(60, 120);
  const under250View = await startLoadingSearch();
  assert.equal(under250View.queryByRole("button", { name: "全选全部", exact: true }), null);
  assert.equal(under250View.queryByTestId("select-all-matching"), null);
  cleanup();

  const script = installBridge(60, 300);
  const view = await startLoadingSearch();

  const selectAllMatching = view.getByRole("button", { name: "全选全部", exact: true }) as HTMLButtonElement;
  assert.equal(selectAllMatching.disabled, true);
  assert.equal(selectAllMatching.parentElement?.getAttribute("title"), "载入完成后可全选");

  fireEvent.click(within(row(view, 2)).getByRole("button", { name: "勾选", exact: true }));
  assert.ok(view.getByText(/^已勾选 1/));

  fireEvent.click(within(row(view, 1)).getByRole("button", { name: "下载", exact: true }));
  const dialog = await view.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(script.taskCalls.length, 0, "the first download of a new search confirms first");
  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(script.taskCalls.length, 1));
  assert.deepEqual(script.taskCalls[0], {
    batchId: LIVE_BATCH,
    revision: 0,
    itemIds: ["s1-item-001"],
    options: { output_root: OUTPUT, concurrency: 3, layout: "sar_dataset_processing", include_precise_orbit: true },
  });

  // Later arrived rows of the same batch go straight to the queue.
  fireEvent.click(within(row(view, 3)).getByRole("button", { name: "下载", exact: true }));
  await waitFor(() => assert.equal(script.taskCalls.length, 2));
  assert.equal(view.queryByRole("dialog", { name: "确认下载任务" }), null);
  assert.deepEqual(script.taskCalls[1].itemIds, ["s1-item-003"]);
});

test("cancel keeps the arrived rows and marks them possibly incomplete", async () => {
  const script = installBridge(60, 120);
  const view = await startLoadingSearch();

  fireEvent.click(view.getByRole("button", { name: "取消检索", exact: true }));
  await waitFor(() => assert.deepEqual(script.cancels, [REQUEST]));
  await waitFor(() => assert.ok(view.getByTestId("result-possibly-incomplete")), { timeout: 3000 });
  assert.equal(view.getByTestId("result-possibly-incomplete").textContent, "未能完整载入");
  assert.ok(view.getByText("60 景", { exact: true }));
  assert.equal(view.queryByTestId("result-loaded-count"), null);
  assert.equal(view.queryByRole("button", { name: "全选全部", exact: true }), null);
  assert.equal(view.queryByRole("button", { name: "取消检索", exact: true }), null);
  assert.ok(row(view, 1));
});

test("a total above 10,000 asks to narrow the search, in English too", async () => {
  installBridge(60, 25_000);
  const view = await startLoadingSearch("en");

  assert.ok(view.getByText("25000 scenes", { exact: true }));
  assert.equal(view.getByTestId("result-loaded-count").textContent, "Loaded 60 / 25000");
  assert.equal(
    view.getByTestId("result-over-ceiling").textContent,
    "25000 scenes match, above the 10000-scene limit. Narrow the area or time range.",
  );
  assert.equal(
    view.getByTestId("sar-over-ceiling-sort-note").textContent,
    "(retaining newest acquisitions first)",
  );
  assert.ok(view.getByRole("button", { name: "Cancel search", exact: true }));
  const selectAll = view.getByRole("button", { name: "Select all", exact: true }) as HTMLButtonElement;
  assert.equal(selectAll.disabled, true);
  assert.equal(selectAll.parentElement?.getAttribute("title"), "Available when loading finishes");
});

test("non-Sentinel-1 SAR sources over ceiling omit the Sentinel-1 newest-first retention note", async () => {
  const ALOS2 = "science.alos-2.palsar-2.scansar-l1.1.v1";
  installBridge(60, 25_000, "complete", ALOS2, "sar");
  window.location.hash = "#/workbench";
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "en");
  const view = render(
    <I18nProvider>
      <DesktopWorkbench dark={false} onToggleDark={() => {}} />
    </I18nProvider>,
  );
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-901"]')));

  fireEvent.click(view.getByTestId("data-source-card"));
  await waitFor(() =>
    assert.ok(view.container.querySelector(`[data-resource-id="${ALOS2}"]`)),
  );
  fireEvent.click(view.container.querySelector(`[data-resource-id="${ALOS2}"]`) as HTMLElement);
  fireEvent.click(view.getByTestId("catalog-confirm-use-btn"));
  await waitFor(() =>
    assert.equal(view.queryByTestId("catalog-confirm-use-btn"), null),
  );

  fireEvent.click(view.getByRole("button", { name: "Search", exact: true }));
  await waitFor(() =>
    assert.equal(
      view.getByTestId("result-over-ceiling").textContent,
      "25000 scenes match, above the 10000-scene limit. Narrow the area or time range.",
    ),
  );
  assert.equal(view.queryByTestId("sar-over-ceiling-sort-note"), null);
});

test("after a restart an interrupted batch keeps its rows and reads possibly incomplete", async () => {
  installBridge(0, null, "interrupted");
  window.location.hash = "#/workbench";
  const view = render(<DesktopWorkbench dark={false} onToggleDark={() => {}} />);
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-901"]')));
  await waitFor(() => assert.ok(view.getByTestId("result-possibly-incomplete")));
  assert.equal(view.queryByTestId("result-loaded-count"), null);
});

test("rapid back-to-back search triggers before re-render invoke searchStart only once (Issue #19)", async () => {
  const script = installBridge(60, 120);
  window.location.hash = "#/workbench";
  const view = render(
    <I18nProvider>
      <DesktopWorkbench dark={false} onToggleDark={() => {}} />
    </I18nProvider>,
  );
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-901"]')));
  const searchBtn = view.getByRole("button", { name: "检索", exact: true });
  const reactPropsKey = Object.keys(searchBtn).find((key) => key.startsWith("__reactProps$"));
  assert.ok(reactPropsKey, "search button must have React props");
  const onClick = (searchBtn as unknown as Record<string, { onClick?: () => void }>)[reactPropsKey]?.onClick;
  assert.equal(typeof onClick, "function");
  await act(async () => {
    onClick!();
    onClick!();
  });
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-001"]')), { timeout: 3000 });
  assert.equal(script.searchStartCalls, 1, "re-entrant search trigger before re-render must not call searchStart twice");
});

test("3-scope selection (current page / loaded / all matching) previews and admits all-match tasks with per-item accounting (#22 B Slice 4)", async () => {
  const script = installBridge(320, 320);
  script.state = "complete";
  window.location.hash = "#/workbench";
  const view = render(
    <I18nProvider>
      <DesktopWorkbench dark={false} onToggleDark={() => {}} />
    </I18nProvider>,
  );
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-901"]')));

  // 1. Current page select-all checks the 2 rows on page 1; <250 scenes hides select-all-matching
  const selectPageBtn = view.getByTestId("select-current-page");
  assert.equal(selectPageBtn.textContent, "全选本页");
  assert.equal(view.queryByTestId("select-all-matching"), null, "<250 scenes does not render 全选全部");

  // Owner 2026-11-16: 「列表」分类切换位于排序栏首位
  const stackToggleBtn = view.getByTestId("toggle-sar-stack-grouping");
  assert.equal(stackToggleBtn.parentElement?.firstElementChild, stackToggleBtn);

  fireEvent.click(selectPageBtn);
  assert.ok(view.getByText(/^已勾选 2/));
  assert.equal(selectPageBtn.textContent, "取消本页全选");

  // Trigger the 320-item search (>=250, already complete) and test full all-match preview + admission loop
  fireEvent.click(view.getByRole("button", { name: "检索", exact: true }));
  await waitFor(() => assert.ok(view.container.querySelector('[data-scene-id="s1-item-001"]')), { timeout: 3000 });
  const selectAllMatchingBtn = await waitFor(() => view.getByTestId("select-all-matching"));
  assert.equal(selectAllMatchingBtn.textContent, "全选全部");

  // Select current page (250 scenes) vs select all matching (320 scenes)
  fireEvent.click(view.getByTestId("select-current-page"));
  assert.ok(view.getByText(/^已勾选 250/));
  fireEvent.click(view.getByTestId("select-all-matching"));
  assert.ok(view.getByText("已选全部匹配 320 景", { exact: true }));

  // Exclude row 2
  fireEvent.click(within(row(view, 2)).getByRole("button", { name: "取消勾选", exact: true }));
  assert.ok(view.getByText("已选全部匹配 319 景（已排除 1 景）", { exact: true }));

  // Click Add to Queue -> calls previewAllMatchTask -> opens confirmation dialog with accounting summary
  fireEvent.click(view.getByRole("button", { name: "加入下载队列", exact: true }));
  const dialog = await view.findByRole("dialog", { name: "确认下载任务" });
  assert.equal(script.allMatchPreviewCalls.length, 1);
  assert.deepEqual(script.allMatchPreviewCalls[0].excluded_item_ids, ["s1-item-002"]);
  const summaryBox = within(dialog).getByTestId("all-match-confirmation-summary");
  assert.ok(summaryBox.textContent?.includes("全部匹配范围：共匹配 319 景，本次将创建 317 景"));
  assert.ok(summaryBox.textContent?.includes("新增入队 317 景 · 进行中跳过 1 景 · 不可下载拒绝 1 景"));

  // Confirm -> calls createAllMatchTask with expected_selection_digest and expected_admissible_count
  fireEvent.click(within(dialog).getByRole("button", { name: "创建下载任务" }));
  await waitFor(() => assert.equal(script.allMatchCreateCalls.length, 1));
  assert.equal(script.allMatchCreateCalls[0].expected_selection_digest, "digest-batch-live-317");
  assert.equal(script.allMatchCreateCalls[0].expected_admissible_count, 317);
  assert.deepEqual(script.allMatchCreateCalls[0].excluded_item_ids, ["s1-item-002"]);
  await waitFor(() =>
    assert.ok(
      view.getByText(
        "已创建全部匹配任务：入队 317 景，跳过进行中 1 景，不可下载拒绝 1 景",
        { exact: true },
      ),
    ),
  );
});

test("manual-clear-results-btn clears active batch, hides possibly_incomplete, prevents re-hydration, and returns to first-use state", async () => {
  const script = installBridge(60, 120);
  const view = await startLoadingSearch();
  fireEvent.click(view.getByRole("button", { name: "取消检索", exact: true }));
  await waitFor(() => assert.ok(view.getByTestId("result-possibly-incomplete")), { timeout: 3000 });
  await waitFor(() => assert.ok(view.getByTestId("manual-clear-results-btn")));

  fireEvent.click(view.getByTestId("manual-clear-results-btn"));

  await waitFor(() => assert.equal(view.container.querySelector("[data-scene-id]"), null));
  assert.equal(view.queryByTestId("result-possibly-incomplete"), null);
  assert.equal(view.queryByText("当前条件下未找到匹配影像"), null);
  assert.ok(view.getByText("已定位 · Test AOI"));
  assert.ok(view.getByText(/确认左侧传感器与时间条件后，即可获取可用影像/));
  await new Promise((r) => setTimeout(r, 80));
  assert.equal(
    view.container.querySelector("[data-scene-id]"),
    null,
    "cleared batch must never auto-rehydrate from stale sourceViews",
  );
  assert.ok(script.cancels.includes(REQUEST));
});
