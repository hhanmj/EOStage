import assert from "node:assert/strict";
import test from "node:test";
import { LandsatAssetMetadataCache } from "./landsatAssetMetadata.ts";
import type { DesktopLandsatAssetDetails } from "./desktopShellBridge.ts";

function details(productKey: string, size = 30): DesktopLandsatAssetDetails {
  return { product_key: productKey, display_id: "LC08_fixture", processing_level: "l2sp",
    total_size_bytes: size, assets: [{ asset_key: "red", file_name: "red.tif", role: "primary", size_bytes: size }] };
}
const tick = () => new Promise<void>(resolve => setImmediate(resolve));

test("visible metadata, inspector, and download share a request without crossing batch identity", async () => {
  const calls: string[] = [];
  const cancelled: string[] = [];
  let finish!: (value: DesktopLandsatAssetDetails) => void;
  const cache = new LandsatAssetMetadataCache({
    load: async request => { calls.push(request.batch_id); return new Promise(resolve => { finish = resolve; }); },
    cancel: id => { cancelled.push(id); }, onChange: () => undefined,
  });
  const visible = cache.acquire("b1", "p1");
  const inspector = cache.acquire("b1", "p1", true);
  const visibleResult = visible.promise.catch(error => error.name);
  visible.release();
  assert.deepEqual(cancelled, [], "the inspector still owns the shared request");
  finish(details("p1"));
  assert.equal(await visibleResult, "AbortError");
  assert.equal((await inspector.promise).total_size_bytes, 30);
  inspector.release();
  const download = cache.acquire("b1", "p1", true);
  assert.equal((await download.promise).total_size_bytes, 30);
  download.release();
  assert.deepEqual(calls, ["b1"]);
  assert.equal(cache.peek("b2", "p1"), null);
  cache.dispose();
});

test("metadata requests are bounded and cancelled in-flight work keeps its slot until settled", async () => {
  const calls: string[] = [];
  const finishes = new Map<string, (value: DesktopLandsatAssetDetails) => void>();
  const cancelled: string[] = [];
  const cache = new LandsatAssetMetadataCache({
    load: request => { calls.push(request.item_id); return new Promise(resolve => finishes.set(request.item_id, resolve)); },
    cancel: id => { cancelled.push(id); }, onChange: () => undefined,
  });
  const handles = ["p1", "p2", "p3"].map(key => cache.acquire("b", key));
  const results = handles.map(handle => handle.promise.catch(error => error.name));
  assert.deepEqual(calls, ["p1", "p2"]);
  handles[0].release();
  assert.equal(cancelled.length, 1);
  assert.deepEqual(calls, ["p1", "p2"], "a local abort does not prove the remote request stopped");
  finishes.get("p1")!(details("p1"));
  await tick();
  assert.deepEqual(calls, ["p1", "p2", "p3"]);
  assert.equal(cache.peek("b", "p1"), null, "late cancelled results are not cached");
  finishes.get("p2")!(details("p2"));
  finishes.get("p3")!(details("p3"));
  await Promise.all(results);
  handles.forEach(handle => handle.release());
  cache.dispose();
});

test("failed or mismatched metadata stays unknown, does not loop, and can be explicitly retried", async () => {
  let calls = 0;
  const cache = new LandsatAssetMetadataCache({
    load: async () => { calls += 1; return details(calls === 1 ? "wrong-product" : "p"); },
    cancel: () => undefined, onChange: () => undefined,
  });
  await assert.rejects(cache.acquire("b", "p").promise, /身份/);
  await assert.rejects(cache.acquire("b", "p").promise);
  assert.equal(calls, 1);
  assert.equal(cache.peek("b", "p"), null);
  assert.equal((await cache.acquire("b", "p", true).promise).total_size_bytes, 30);
  assert.equal(calls, 2);
  cache.dispose();
});

test("cache is bounded and rejects totals that do not equal the returned file set", async () => {
  const cache = new LandsatAssetMetadataCache({
    load: async request => request.item_id === "bad"
      ? { ...details("bad"), total_size_bytes: 100 }
      : details(request.item_id),
    cancel: () => undefined, onChange: () => undefined,
  }, { capacity: 2 });
  for (const key of ["p1", "p2", "p3"]) await cache.acquire("b", key).promise;
  assert.equal(cache.peek("b", "p1"), null);
  assert.equal(cache.peek("b", "p3")?.total_size_bytes, 30);
  await assert.rejects(cache.acquire("b", "bad").promise, /大小/);
  cache.dispose();
});

test("foreground requests pass queued background work and disposing prevents late publication", async () => {
  const calls: string[] = [];
  const finish = new Map<string, (value: DesktopLandsatAssetDetails) => void>();
  let published = 0;
  const cache = new LandsatAssetMetadataCache({
    load: request => { calls.push(request.item_id); return new Promise(resolve => finish.set(request.item_id, resolve)); },
    cancel: () => undefined, onChange: () => { published += 1; },
  });
  const handles = [cache.acquire("b", "p1"), cache.acquire("b", "p2"), cache.acquire("b", "p3"), cache.acquire("b", "p4", true)];
  const settled = handles.map(handle => handle.promise.catch(error => error.name));
  finish.get("p1")!(details("p1"));
  await tick();
  assert.deepEqual(calls, ["p1", "p2", "p4"]);
  cache.dispose();
  assert.equal(await settled[2], "AbortError", "a queued request is cancelled without starting");
  finish.get("p2")!(details("p2"));
  finish.get("p4")!(details("p4"));
  await Promise.all(settled);
  await tick();
  assert.equal(published, 1, "disposed requests never publish a late result");
  const remounted = cache.acquire("b2", "p5");
  finish.get("p5")!(details("p5"));
  assert.equal((await remounted.promise).total_size_bytes, 30, "StrictMode effect cleanup does not permanently disable the cache");
  cache.dispose();
});

test("previously measured size remains displayable after TTL and reacquisition revalidates it", async context => {
  let now = 1_000;
  let calls = 0;
  context.mock.method(Date, "now", () => now);
  const cache = new LandsatAssetMetadataCache({
    load: async request => { calls += 1; return details(request.item_id, calls * 30); },
    cancel: () => undefined, onChange: () => undefined,
  });
  await cache.acquire("b", "p").promise;
  now += 300_001;
  assert.equal(cache.peek("b", "p")?.total_size_bytes, 30, "a render must not erase measured metadata solely because it aged");
  assert.equal((await cache.acquire("b", "p").promise).total_size_bytes, 60);
  assert.equal(calls, 2);
  assert.equal(cache.peek("another-batch", "p"), null);
  cache.dispose();
});
