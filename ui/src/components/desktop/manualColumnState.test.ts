import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_MANUAL_COLUMNS,
  collapseManualColumn,
  expandManualColumn,
  parseManualColumnState,
  resetManualSplitter,
  resizeManualColumns,
  resolveManualColumnLayout,
} from "./manualColumnState.ts";

function fresh() {
  return structuredClone(DEFAULT_MANUAL_COLUMNS);
}

test("default layout gives search 320, results 440 and the remainder to the map", () => {
  assert.deepEqual(resolveManualColumnLayout(fresh(), 1440), {
    collapsed: { search: false, map: false, results: false },
    widths: { search: 320, map: 664, results: 440 },
  });
});

test("widths clamp to their scientific-panel limits and map remains unfolded", () => {
  const state = fresh();
  state.widths.search = 999;
  state.widths.results = 100;
  // 1440 - 16 = 1424 usable; search clamped to 480; results clamped to min 340; map = 1424 - 480 - 340 = 604
  assert.deepEqual(resolveManualColumnLayout(state, 1440).widths, { search: 480, map: 604, results: 340 });
  const narrow = resolveManualColumnLayout(state, 1200);
  assert.equal(narrow.collapsed.map, false);
  assert.ok(narrow.widths.map >= 360);
});

test("collapsed search or results columns have 0 width and expand via handles", () => {
  const state = fresh();
  state.collapsed.search = true;
  const layoutSearchCollapsed = resolveManualColumnLayout(state, 1440);
  assert.equal(layoutSearchCollapsed.widths.search, 0);
  assert.equal(layoutSearchCollapsed.widths.results, 440);
  assert.equal(layoutSearchCollapsed.widths.map, 1424 - 440);

  const stateResultsCollapsed = fresh();
  stateResultsCollapsed.collapsed.results = true;
  const layoutResultsCollapsed = resolveManualColumnLayout(stateResultsCollapsed, 1440);
  assert.equal(layoutResultsCollapsed.widths.results, 0);
  assert.equal(layoutResultsCollapsed.widths.search, 320);
  assert.equal(layoutResultsCollapsed.widths.map, 1424 - 320);

  const expanded = expandManualColumn(state, "search");
  assert.equal(expanded.collapsed.search, false);
});

test("splitters resize smoothly without collapsing the map", () => {
  // search-map: dragging right increases search width
  const state = resizeManualColumns(fresh(), "search-map", 80, 1440);
  assert.equal(state.widths.search, 400);
  assert.equal(resolveManualColumnLayout(state, 1440).widths.map, 584);
  assert.equal(state.collapsed.map, false);

  // map-results: dragging right decreases results width (- delta)
  const resultRight = resizeManualColumns(fresh(), "map-results", 40, 1440);
  assert.equal(resultRight.widths.results, 400); // 440 - 40 = 400
  assert.equal(resultRight.collapsed.map, false);

  // map-results: dragging left increases results width
  const resultLeft = resizeManualColumns(fresh(), "map-results", -60, 1440);
  assert.equal(resultLeft.widths.results, 500); // 440 - (-60) = 500
  assert.equal(resultLeft.collapsed.map, false);
});

test("drag smoothly clamps within bounds without collapsing the map", () => {
  // Excessive dragging to the right clamps search to max without collapsing map
  const searchExcess = resizeManualColumns(fresh(), "search-map", 500, 1440);
  assert.equal(searchExcess.widths.search, 480);
  assert.equal(searchExcess.collapsed.map, false);

  // Excessive dragging to the left clamps results without collapsing map
  const resultExcess = resizeManualColumns(fresh(), "map-results", -800, 1440);
  assert.ok(resultExcess.widths.results <= 1440 - 320 - 360 - 16);
  assert.equal(resultExcess.collapsed.map, false);
});

test("double-click reset restores defaults", () => {
  const state = fresh();
  state.widths.search = 460;
  const reset = resetManualSplitter(state, "search-map");
  assert.equal(reset.widths.search, 320);
  assert.equal(reset.collapsed.search, false);
  assert.equal(reset.collapsed.map, false);

  state.widths.results = 520;
  const resetResults = resetManualSplitter(state, "map-results");
  assert.equal(resetResults.widths.results, 440);
  assert.equal(resetResults.collapsed.results, false);
});

test("persisted widths and collapsed state parse with invalid values safely defaulted", () => {
  const parsed = parseManualColumnState(JSON.stringify({
    collapsed: { search: true, map: false, results: false },
    widths: { search: 425, results: 515 },
  }));
  assert.equal(parsed.collapsed.search, true);
  assert.deepEqual(parsed.widths, { search: 425, results: 515 });
  assert.equal(parseManualColumnState("{invalid").widths.results, 440);
  assert.equal(parseManualColumnState(JSON.stringify({ collapsed: {}, widths: { search: 1, results: "wide" } })).widths.search, 320);
  assert.deepEqual(parseManualColumnState(JSON.stringify({ collapsed: { search: true, map: true, results: true } })), fresh());
});

test("small-screen (920px window -> 868px available) and large-screen (2560px window) layouts never overflow horizontally", () => {
  const small = resolveManualColumnLayout(fresh(), 868);
  assert.equal(small.collapsed.map, false);
  assert.ok(small.widths.search >= 180);
  assert.ok(small.widths.map >= 160);
  assert.ok(small.widths.results >= 200);
  assert.ok(
    small.widths.search + small.widths.map + small.widths.results + 16 <= 868,
    `expected small layout <= 868px, got ${small.widths.search + small.widths.map + small.widths.results + 16}`,
  );

  // Narrow focus mode (920x620 window -> ~788px container, search auto-collapsed):
  // map canvas stays usable (>= 280px) alongside readable results panel (>= 340px)
  const narrowSearchCollapsed = resolveManualColumnLayout(collapseManualColumn(fresh(), "search"), 788);
  assert.equal(narrowSearchCollapsed.collapsed.search, true);
  assert.equal(narrowSearchCollapsed.collapsed.results, false);
  assert.ok(narrowSearchCollapsed.widths.map >= 280, `expected map >= 280, got ${narrowSearchCollapsed.widths.map}`);
  assert.ok(narrowSearchCollapsed.widths.results >= 340, `expected results >= 340, got ${narrowSearchCollapsed.widths.results}`);
  assert.ok(narrowSearchCollapsed.widths.map + narrowSearchCollapsed.widths.results + 16 <= 788);

  const large = resolveManualColumnLayout(fresh(), 2508);
  assert.equal(large.widths.search, 320);
  assert.equal(large.widths.results, 440);
  assert.equal(large.widths.map, 2508 - 16 - 320 - 440);
});
