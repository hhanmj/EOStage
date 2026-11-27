import assert from "node:assert/strict";
import test from "node:test";

import {
  LandsatAssetDetailsCoordinator,
  landsatAssetDisplayName,
  landsatAssetRoleLabel,
  type LandsatAssetDetailsState,
} from "./landsatAssetDetails.ts";
import type { DesktopLandsatAssetDetails } from "./desktopShellBridge.ts";

function details(productKey: string): DesktopLandsatAssetDetails {
  return {
    product_key: productKey,
    display_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
    processing_level: "l2sp",
    total_size_bytes: 30,
    assets: [
      { asset_key: "red", file_name: "red.tif", role: "primary", size_bytes: 20 },
      { asset_key: "mtl.xml", file_name: "mtl.xml", role: "metadata", size_bytes: 10 },
    ],
  };
}

test("Landsat asset detail labels keep scientific bands and support files distinct", () => {
  assert.equal(landsatAssetDisplayName("red"), "红光 B4");
  assert.equal(landsatAssetDisplayName("qa_pixel"), "像元质量 QA");
  assert.equal(landsatAssetDisplayName("custom"), "custom");
  assert.equal(landsatAssetRoleLabel("primary"), "科学波段");
  assert.equal(landsatAssetRoleLabel("auxiliary"), "辅助文件");
  assert.equal(landsatAssetRoleLabel("metadata"), "元数据");
});

test("asset detail coordinator cancels old requests and ignores late results", async () => {
  const states: LandsatAssetDetailsState[] = [];
  const cancelled: string[] = [];
  let resolveFirst: ((value: DesktopLandsatAssetDetails) => void) | null = null;
  const coordinator = new LandsatAssetDetailsCoordinator((state) => states.push(state));
  const first = coordinator.request(
    { productKey: "p1", batchId: "b1", requestId: "r1" },
    () => new Promise((resolve) => { resolveFirst = resolve; }),
    () => cancelled.push("r1"),
  );
  const second = coordinator.request(
    { productKey: "p2", batchId: "b2", requestId: "r2" },
    async () => details("p2"),
    () => cancelled.push("r2"),
  );
  await second;
  resolveFirst?.(details("p1"));
  await first;

  assert.deepEqual(cancelled, ["r1"]);
  assert.equal(states.at(-1)?.status, "available");
  assert.equal(states.at(-1)?.status === "available" ? states.at(-1)?.productKey : null, "p2");
});

test("asset detail coordinator fail-closes identity mismatch and cancel clears state", async () => {
  const states: LandsatAssetDetailsState[] = [];
  let cancelled = 0;
  const coordinator = new LandsatAssetDetailsCoordinator((state) => states.push(state));
  await coordinator.request(
    { productKey: "p1", batchId: "b1", requestId: "r1" },
    async () => details("wrong"),
    () => { cancelled += 1; },
  );
  assert.equal(states.at(-1)?.status, "error");
  assert.match(states.at(-1)?.status === "error" ? states.at(-1)?.error ?? "" : "", /身份/);

  void coordinator.request(
    { productKey: "p2", batchId: "b2", requestId: "r2" },
    () => new Promise(() => undefined),
    () => { cancelled += 1; },
  );
  coordinator.cancel();
  assert.equal(cancelled, 1);
  assert.equal(states.at(-1), null);
});
