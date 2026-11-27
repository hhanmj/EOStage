import assert from "node:assert/strict";
import test from "node:test";

import {
  PreviewSaveDirectorySession,
  previewProductFileBase,
  resolvePreviewSaveBatchId,
} from "./previewSave.ts";

test("Preview directories are reused only within one immutable source batch", () => {
  const session = new PreviewSaveDirectorySession();

  session.remember("batch-a", "D:/preview/a");

  assert.equal(session.get("batch-a"), "D:/preview/a");
  assert.equal(session.get("batch-b"), null);
  session.forget("batch-a");
  assert.equal(session.get("batch-a"), null);
});

test("empty batch IDs and directories are never remembered", () => {
  const session = new PreviewSaveDirectorySession();

  session.remember("", "D:/preview/a");
  session.remember("batch-a", "   ");

  assert.equal(session.get(""), null);
  assert.equal(session.get("batch-a"), null);
});

test("task Preview save uses the source SearchBatch identity", () => {
  assert.equal(
    resolvePreviewSaveBatchId("current-search", {
      batch_id: "task-delivery-batch",
      source_batch_id: "source-search-batch",
    }),
    "source-search-batch",
  );
  assert.equal(
    resolvePreviewSaveBatchId("current-search", { batch_id: "source-only" }),
    "source-only",
  );
  assert.equal(resolvePreviewSaveBatchId("current-search", null), "current-search");
});

test("Preview product file bases are stable and filesystem safe", () => {
  assert.equal(previewProductFileBase("S1A/IW:GRD?P1.zip"), "S1A_IW_GRD_P1");
  assert.equal(previewProductFileBase("  .  "), "product");
  assert.equal(previewProductFileBase("P1.jpg"), "P1");
});
