import assert from "node:assert/strict";
import test from "node:test";

import { catalogSearchCountSummary, possiblyIncompleteLabel } from "./catalogSearchCountSummary.ts";

test("bounded search summary uses product language and omits provider implementation details", () => {
  assert.deepEqual(catalogSearchCountSummary({
    targetCount: 40,
    providerMatchedCount: 120,
    providerReturnedCount: 40,
    frozenCandidateCount: 40,
    currentPageCount: 12,
    checkedCount: 3,
    truncated: false,
  }), {
    target: "目标 40",
    matched: "候选总量 120",
    frozen: "已载入 40",
    current: "当前显示 12",
    selected: "已选择 3",
  });
});

test("old batches without search stats show unknown provider matches and only their current returned rows", () => {
  assert.deepEqual(catalogSearchCountSummary({
    targetCount: null,
    providerMatchedCount: null,
    providerReturnedCount: null,
    frozenCandidateCount: 8,
    currentPageCount: 3,
    checkedCount: 1,
    truncated: false,
  }), {
    target: null,
    matched: "候选总量 未知",
    frozen: "已载入 8",
    current: "当前显示 3",
    selected: "已选择 1",
  });
});

test("a bounded search with no provider total reports only its admitted count", () => {
  assert.equal(catalogSearchCountSummary({
    targetCount: 20,
    providerMatchedCount: null,
    providerReturnedCount: 20,
    frozenCandidateCount: 20,
    currentPageCount: 8,
    checkedCount: 0,
    truncated: false,
  }).matched, null);
});

test("all-results mode names the explicit safety ceiling instead of pretending it is the total", () => {
  assert.equal(catalogSearchCountSummary({
    targetCount: null,
    providerMatchedCount: 12_480,
    providerReturnedCount: 10_000,
    frozenCandidateCount: 10_000,
    currentPageCount: 24,
    checkedCount: 0,
    truncated: true,
  }).matched, "候选总量 12480 · 已达到安全上限 10000");
});

test("an incomplete all-results import below the safety ceiling is not mislabeled as a ceiling hit", () => {
  assert.equal(catalogSearchCountSummary({
    targetCount: null,
    providerMatchedCount: 450,
    providerReturnedCount: 440,
    frozenCandidateCount: 440,
    currentPageCount: 24,
    checkedCount: 0,
    truncated: true,
  }).matched, "候选总量 450 · 未能完整载入");
});

test("possibly incomplete batches reuse the existing wording at and below the safety ceiling", () => {
  assert.equal(possiblyIncompleteLabel(250), "未能完整载入");
  assert.equal(possiblyIncompleteLabel(9_999), "未能完整载入");
  assert.equal(possiblyIncompleteLabel(10_000), "已达到安全上限 10000");
});
