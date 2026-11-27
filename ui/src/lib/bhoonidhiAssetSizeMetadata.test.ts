import assert from "node:assert/strict";
import test from "node:test";

import { BhoonidhiAssetSizeCache } from "./bhoonidhiAssetSizeMetadata.ts";

test("Bhoonidhi size cache limits concurrent probes to two and validates identity", async () => {
  let running = 0;
  let maxRunning = 0;
  const releases: (() => void)[] = [];
  const cache = new BhoonidhiAssetSizeCache({
    load: async request => {
      running += 1;
      maxRunning = Math.max(maxRunning, running);
      await new Promise<void>(resolve => releases.push(resolve));
      running -= 1;
      return { product_key: request.item_id, size_bytes: 1024 };
    },
    cancel: () => undefined,
    onChange: () => undefined,
  });
  const handles = ["a", "b", "c"].map(key => cache.acquire("batch", key));
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(maxRunning, 2);
  releases.shift()?.();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(maxRunning, 2);
  while (releases.length) releases.shift()?.();
  await Promise.all(handles.map(handle => handle.promise));
  assert.equal(cache.peek("batch", "a")?.size_bytes, 1024);
  cache.dispose();
});

test("expired size evidence is removed consistently from peek and status", async () => {
  const originalNow = Date.now;
  let now = 10_000;
  Date.now = () => now;
  try {
    const cache = new BhoonidhiAssetSizeCache({
      load: async request => ({ product_key: request.item_id, size_bytes: 4096 }),
      cancel: () => undefined,
      onChange: () => undefined,
    });
    const handle = cache.acquire("batch", "product");
    await handle.promise;
    assert.equal(cache.peek("batch", "product")?.size_bytes, 4096);
    assert.equal(cache.status("batch", "product"), "available");

    now += 300_001;
    assert.equal(cache.peek("batch", "product"), null);
    assert.equal(cache.status("batch", "product"), null);
    cache.dispose();
  } finally {
    Date.now = originalNow;
  }
});

test("releasing the last viewport consumer cancels an active probe", async () => {
  const cancelled: string[] = [];
  let resolve!: (value: { product_key: string; size_bytes: number }) => void;
  const cache = new BhoonidhiAssetSizeCache({
    load: request => new Promise(value => { resolve = value; }),
    cancel: requestId => cancelled.push(requestId),
    onChange: () => undefined,
  });
  const handle = cache.acquire("batch", "product");
  await new Promise(resolveTick => setTimeout(resolveTick, 0));
  handle.release();
  await assert.rejects(handle.promise, { name: "AbortError" });
  assert.equal(cancelled.length, 1);
  resolve({ product_key: "product", size_bytes: 2048 });
  await new Promise(resolveTick => setTimeout(resolveTick, 0));
  cache.dispose();
});
