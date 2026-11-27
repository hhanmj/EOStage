import assert from "node:assert/strict";
import test from "node:test";

import { nisarSubsetRunningPhase, nisarSubsetRunningPhaseLabel } from "./nisarSubsetTaskPhase.ts";

const progress = (transferring: number, idle: number, without_progress: number) => ({ transferring, idle, without_progress });

test("a resumed subset task shows verification before its first new byte", () => {
  const status = { active_downloads: [{ bytes: 0 }], active_progress: progress(0, 0, 1), reusable_verified_bytes: 4096 };
  assert.equal(nisarSubsetRunningPhase(status), "verifying");
  assert.equal(nisarSubsetRunningPhaseLabel(status), "正在复核已下载分段");
  // A host without the phase facts, before any progress row.
  assert.equal(nisarSubsetRunningPhase({ active_downloads: [], reusable_verified_bytes: 4096 }), "verifying");
});

test("a fresh subset task shows preparation before its first byte", () => {
  assert.equal(nisarSubsetRunningPhaseLabel({ active_progress: progress(0, 0, 1), reusable_verified_bytes: null }), "正在准备裁剪任务");
  assert.equal(nisarSubsetRunningPhase({ active_downloads: [{ bytes: 0 }] }), "preparing");
});

test("an active item with a positive transfer rate is a Range transfer", () => {
  assert.equal(
    nisarSubsetRunningPhaseLabel({ active_downloads: [{ bytes: 1024 }], active_progress: progress(1, 0, 0), reusable_verified_bytes: 4096 }),
    "正在传输裁剪范围",
  );
  // A host without the phase facts still reads arriving bytes as a transfer.
  assert.equal(nisarSubsetRunningPhase({ active_downloads: [{ bytes: 1024 }] }), "transferring");
});

test("the zero-rate progress row published at the end of the science transfer is the output phase", () => {
  assert.equal(nisarSubsetRunningPhaseLabel({ active_downloads: [{ bytes: 2048 }], active_progress: progress(0, 1, 0) }), "正在生成裁剪结果");
  // A continue whose science was already complete transfers nothing in this run.
  assert.equal(
    nisarSubsetRunningPhase({ active_downloads: [{ bytes: 0 }], active_progress: progress(0, 1, 0), reusable_verified_bytes: 4096 }),
    "writing",
  );
});

test("review P2-2: a progress total no longer marks the output phase", () => {
  // The former rule read "bytes reached the total at zero rate" as writing, which
  // required a sticky persisted total; only the Store phase facts count now.
  const reachedTotal = { active_downloads: [{ bytes: 2048, expected_size: 2048 }], bytes_per_second: 0 };
  assert.equal(nisarSubsetRunningPhase({ ...reachedTotal, active_progress: progress(1, 0, 0) }), "transferring");
  assert.equal(nisarSubsetRunningPhase(reachedTotal), "transferring");
});

test("downloader D1: an item waiting for the output writer reads 等待写出", () => {
  const waiting = { without_progress: 0, transferring: 0, idle: 0, waiting: 1 };
  assert.equal(nisarSubsetRunningPhase({ active_progress: waiting, reusable_verified_bytes: 4096 }), "waiting");
  assert.equal(nisarSubsetRunningPhaseLabel({ active_progress: waiting, reusable_verified_bytes: 4096 }), "等待写出");
  assert.equal(
    nisarSubsetRunningPhaseLabel({ active_progress: { without_progress: 0, transferring: 0, idle: 1, waiting: 2 } }),
    "正在生成裁剪结果（另有 2 项等待写出）",
  );
  assert.equal(
    nisarSubsetRunningPhaseLabel({ active_progress: { without_progress: 0, transferring: 1, idle: 0, waiting: 1 } }),
    "正在传输裁剪范围（另有 1 项等待写出）",
  );
  assert.equal(
    nisarSubsetRunningPhaseLabel({ active_progress: { without_progress: 0, transferring: 1, idle: 1, waiting: 1 } }),
    "正在传输裁剪范围（另有 1 项正在生成裁剪结果，1 项等待写出）",
  );
  // A host without the waiting fact keeps its labels.
  assert.equal(nisarSubsetRunningPhaseLabel({ active_progress: progress(0, 1, 0) }), "正在生成裁剪结果");
});

test("several active items share one label by precedence and count the items already writing", () => {
  assert.equal(
    nisarSubsetRunningPhaseLabel({ active_downloads: [{ bytes: 10 }], active_progress: progress(1, 2, 0) }),
    "正在传输裁剪范围（另有 2 项正在生成裁剪结果）",
  );
  assert.equal(nisarSubsetRunningPhaseLabel({ active_progress: progress(0, 1, 1), reusable_verified_bytes: 9 }), "正在生成裁剪结果");
  assert.equal(nisarSubsetRunningPhaseLabel({ active_progress: progress(0, 0, 3), reusable_verified_bytes: 9 }), "正在复核已下载分段");
});
