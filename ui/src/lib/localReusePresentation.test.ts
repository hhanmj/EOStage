import assert from "node:assert/strict";
import test from "node:test";

import { presentLocalReuse, reacquireConfirmationText } from "./localReusePresentation.ts";

test("a reuse without content evidence says so and offers 重新下载", () => {
  const presentation = presentLocalReuse(
    { disposition: "reused", grade: "size_and_format", conflict_reason: null, conflict_file_name: null },
    "skipped",
  );
  assert.equal(presentation?.label, "复用已有文件（未做内容校验）");
  assert.equal(presentation?.offerReacquire, true);
});

test("a reuse with content evidence names its evidence and offers no reacquire", () => {
  for (const [grade, label] of [
    ["service_checksum", "复用已有文件（已核对服务方校验值）"],
    ["ledger_identity", "复用已有文件（与上次下载记录一致）"],
    ["ledger_rehash", "复用已有文件（已核对上次下载的 SHA-256）"],
  ] as const) {
    const presentation = presentLocalReuse(
      { disposition: "reused", grade, conflict_reason: null, conflict_file_name: null },
      "skipped",
    );
    assert.equal(presentation?.label, label);
    assert.equal(presentation?.offerReacquire, false);
  }
});

test("a renamed conflict names the kept copy and why", () => {
  const decision = {
    disposition: "renamed_conflict" as const,
    grade: null,
    conflict_reason: "size_mismatch",
    conflict_file_name: "scene.zip.eostage-conflict-20261020T010203Z",
  };
  const completed = presentLocalReuse(decision, "completed");
  assert.equal(completed?.label, "已下载 · 原文件已另存为 scene.zip.eostage-conflict-20261020T010203Z");
  assert.equal(completed?.detail, "原文件大小与服务方不符");
  assert.equal(completed?.offerReacquire, false);
  const pending = presentLocalReuse(decision, "queued");
  assert.equal(pending?.label, "原文件已另存为 scene.zip.eostage-conflict-20261020T010203Z");
  const reacquired = presentLocalReuse({ ...decision, conflict_reason: "reacquire" }, "completed");
  assert.equal(reacquired?.detail, "按你的要求重新下载");
});

test("a requested reacquisition and no decision", () => {
  assert.equal(
    presentLocalReuse(
      { disposition: "reacquire_requested", grade: null, conflict_reason: null, conflict_file_name: null },
      "queued",
    )?.label,
    "将重新下载，原文件将另存保留",
  );
  assert.equal(presentLocalReuse(null, "completed"), null);
});

test("the reacquire confirmation states the kept copy and the user's choice", () => {
  assert.equal(
    reacquireConfirmationText("scene.zip"),
    "将重新下载此项。原文件会另存为 scene.zip.eostage-conflict-…，下载完成后由您决定是否删除。",
  );
});
