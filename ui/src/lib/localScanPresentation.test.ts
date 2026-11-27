import assert from "node:assert/strict";
import test from "node:test";

import { localScanProgressText } from "./localScanPresentation.ts";

test("the task card names the running scan and the reuse it decided", () => {
  assert.equal(
    localScanProgressText({ state: "scanning", scanned_items: 12, total_items: 250, reused_count: 0, unverified_count: 0 }),
    "正在扫描已有文件 12/250",
  );
  assert.equal(
    localScanProgressText({ state: "scanning", scanned_items: 40, total_items: 250, reused_count: 31, unverified_count: 4 }),
    "正在扫描已有文件 40/250 · 已复用 31（其中 4 个未做内容校验）",
  );
  assert.equal(
    localScanProgressText({ state: "done", scanned_items: 250, total_items: 250, reused_count: 31, unverified_count: 0 }),
    "已复用 31",
  );
  assert.equal(
    localScanProgressText({ state: "none", scanned_items: 0, total_items: 0, reused_count: 2, unverified_count: 2 }),
    "已复用 2（其中 2 个未做内容校验）",
  );
});

test("a finished scan without reuse adds nothing to the card", () => {
  assert.equal(localScanProgressText(null), null);
  assert.equal(
    localScanProgressText({ state: "done", scanned_items: 5, total_items: 5, reused_count: 0, unverified_count: 0 }),
    null,
  );
  assert.equal(
    localScanProgressText({ state: "stopped", scanned_items: 3, total_items: 5, reused_count: 0, unverified_count: 0 }),
    null,
  );
});
