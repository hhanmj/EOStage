import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_TASK_PAGE_SIZE,
  DEFAULT_WORKSPACE_PAGE_SIZE,
  MAX_CATALOG_PAGE_ITEMS,
  SCENE_WORKSPACE_ROW_HEIGHT,
  catalogPageCount,
  catalogPageItems,
  catalogPaginationTotal,
  pageIndexFromScrollTop,
  scrollTopFromPageIndex,
} from "./workspaceViewState.ts";

test("catalog workspace pages round-trip at the 250-item presentation capacity", () => {
  assert.equal(DEFAULT_WORKSPACE_PAGE_SIZE, 250);
  assert.equal(MAX_CATALOG_PAGE_ITEMS, 250);
  assert.equal(DEFAULT_TASK_PAGE_SIZE, 100);
  const pageHeight = DEFAULT_WORKSPACE_PAGE_SIZE * SCENE_WORKSPACE_ROW_HEIGHT;
  assert.equal(pageIndexFromScrollTop(0), 0);
  assert.equal(pageIndexFromScrollTop(pageHeight - 1), 0);
  assert.equal(pageIndexFromScrollTop(pageHeight), 1);
  assert.equal(pageIndexFromScrollTop(pageHeight * 7 + 20), 7);
  assert.equal(scrollTopFromPageIndex(7), pageHeight * 7);
});

test("600 frozen results paginate as 250 / 250 / 100 independently of remote total", () => {
  const remoteTotal = 975;
  const total = catalogPaginationTotal({ workingSetCount: 600, filteredCount: null });
  assert.equal(total, 600);
  assert.equal(catalogPageCount(total), 3);
  assert.deepEqual([0, 1, 2].map((page) => catalogPageItems(total, page)), [250, 250, 100]);
  assert.equal(remoteTotal, 975);
});

test("137 actual results stay one page and a 73-row local filter becomes the page basis", () => {
  assert.equal(catalogPageCount(catalogPaginationTotal({ workingSetCount: 137, filteredCount: null })), 1);
  const filteredTotal = catalogPaginationTotal({ workingSetCount: 600, filteredCount: 73 });
  assert.equal(filteredTotal, 73);
  assert.equal(catalogPageCount(filteredTotal), 1);
  assert.equal(catalogPageItems(filteredTotal, 0), 73);
});

test("restored virtual workspace positions clamp invalid numeric input", () => {
  assert.equal(pageIndexFromScrollTop(-10), 0);
  assert.equal(pageIndexFromScrollTop(Number.NaN), 0);
  assert.equal(scrollTopFromPageIndex(-3), 0);
  assert.equal(scrollTopFromPageIndex(Number.POSITIVE_INFINITY), 0);
});
