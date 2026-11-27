import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { SceneRow } from "../../lib/bridge.ts";
import {
  CSP_ALLOWED_IMAGE_HOSTS,
  thumbnailUrlFor,
  isAllowedByCsp,
  createThumbnailQueue,
  placeholderReason,
} from "./thumbnailLogic.ts";

test("W5-3c: Thumbnails in 48x48 Slot Suite", async (t) => {
  // 1. thumbnailUrlFor 提取规则
  await t.test("1. thumbnailUrlFor prioritizes thumbnail over browse and requires absolute safe URL", () => {
    // (a) thumbnail 角色优先于 browse
    const rowWithBoth: Partial<SceneRow> = {
      assets: [
        {
          role: "browse",
          url: "https://nisar.asf.earthdatacloud.nasa.gov/browse.jpg",
        },
        {
          role: "thumbnail",
          url: "https://nisar.asf.earthdatacloud.nasa.gov/thumb.jpg",
        },
      ],
    };
    assert.equal(
      thumbnailUrlFor(rowWithBoth),
      "https://nisar.asf.earthdatacloud.nasa.gov/thumb.jpg",
      "应优先提取 thumbnail 角色"
    );

    // (b) 仅有 browse 角色时退回使用 browse
    const rowWithBrowse: Partial<SceneRow> = {
      assets: [
        {
          role: "primary",
          url: "https://nisar.asf.earthdatacloud.nasa.gov/data.h5",
        },
        {
          role: "browse",
          url: "https://nisar.asf.earthdatacloud.nasa.gov/browse.png",
        },
      ],
    };
    assert.equal(
      thumbnailUrlFor(rowWithBrowse),
      "https://nisar.asf.earthdatacloud.nasa.gov/browse.png"
    );

    // (c) 非 https/data/blob 相对路径或不合法 URL 返回 null
    const rowWithRelative: Partial<SceneRow> = {
      assets: [
        {
          role: "thumbnail",
          url: "/relative/path/thumb.png",
        },
      ],
    };
    assert.equal(thumbnailUrlFor(rowWithRelative), null);

    // (d) 无 thumbnail/browse 或 assets 为空时返回 null
    assert.equal(thumbnailUrlFor({ assets: [] }), null);
    assert.equal(thumbnailUrlFor({}), null);
    assert.equal(thumbnailUrlFor(null), null);

    // (e) 兼容从 native_metadata.assets 中提取
    const rowFromMetadata: Partial<SceneRow> = {
      native_metadata: {
        assets: [
          {
            role: "thumbnail",
            url: "https://nisar.asf.earthdatacloud.nasa.gov/meta_thumb.jpg",
          },
        ],
      },
    };
    assert.equal(
      thumbnailUrlFor(rowFromMetadata),
      "https://nisar.asf.earthdatacloud.nasa.gov/meta_thumb.jpg"
    );

    // (f) row.assets 为空数组时仍回退读取 native_metadata.assets：
    // ASF Sentinel-1 冻结行的官方 Browse URL 存在于冻结元数据中。
    const rowEmptyAssetsWithMetadata: Partial<SceneRow> = {
      assets: [],
      native_metadata: {
        assets: [
          {
            role: "primary",
            url: null,
          },
          {
            role: "browse",
            url: "https://datapool.asf.alaska.edu/BROWSE/SA/S1A_IW_GRDH_1SDV_20260101T000000.jpg",
          },
        ],
      },
    };
    assert.equal(
      thumbnailUrlFor(rowEmptyAssetsWithMetadata),
      "https://datapool.asf.alaska.edu/BROWSE/SA/S1A_IW_GRDH_1SDV_20260101T000000.jpg",
      "assets 为空数组时必须回退到 native_metadata.assets"
    );

    // (g) NISAR 官方预览资产的角色是 "preview"，作为第三优先级被接受
    const nisarPreviewRow: Partial<SceneRow> = {
      assets: [
        {
          role: "primary",
          url: null,
        },
        {
          role: "preview",
          url: "https://nisar.asf.earthdatacloud.nasa.gov/browse/NISAR_L1_PR_RSLC001.png",
        },
      ],
    };
    assert.equal(
      thumbnailUrlFor(nisarPreviewRow),
      "https://nisar.asf.earthdatacloud.nasa.gov/browse/NISAR_L1_PR_RSLC001.png",
      "preview 角色必须被接受（NISAR 官方 LATLON 预览）"
    );

    // (g2) 冻结批次里的原始 NISAR 资产没有 url，地址在 https routes 里；s3 路由不算
    const frozenNisarRow = {
      assets: [
        {
          role: "thumbnail",
          routes: [
            { scheme: "s3", locator: "s3://bucket/thumb.png" },
            { scheme: "https", locator: "https://nisar.asf.earthdatacloud.nasa.gov/BROWSE/x_LATLON_thumbnail.png" },
          ],
        },
      ],
    } as unknown as Partial<SceneRow>;
    assert.equal(
      thumbnailUrlFor(frozenNisarRow),
      "https://nisar.asf.earthdatacloud.nasa.gov/BROWSE/x_LATLON_thumbnail.png",
    );
    const s3OnlyRow = {
      assets: [{ role: "thumbnail", routes: [{ scheme: "s3", locator: "s3://bucket/thumb.png" }] }],
    } as unknown as Partial<SceneRow>;
    assert.equal(thumbnailUrlFor(s3OnlyRow), null);

    // (h) 行内 assets 优先于 native_metadata.assets
    const rowBothSources: Partial<SceneRow> = {
      assets: [
        {
          role: "thumbnail",
          url: "https://bhoonidhi.nrsc.gov.in/thumb_row.jpg",
        },
      ],
      native_metadata: {
        assets: [
          {
            role: "thumbnail",
            url: "https://cumulus.asf.earthdatacloud.nasa.gov/thumb_meta.jpg",
          },
        ],
      },
    };
    assert.equal(
      thumbnailUrlFor(rowBothSources),
      "https://bhoonidhi.nrsc.gov.in/thumb_row.jpg",
      "行内任务冻结资产优先于冻结批次元数据"
    );

    // (i) 两种来源都没有有效快视图 URL 时仍返回 null
    const rowNoUsable: Partial<SceneRow> = {
      assets: [{ role: "primary", url: "https://datapool.asf.alaska.edu/data.zip" }],
      native_metadata: { assets: [] },
    };
    assert.equal(thumbnailUrlFor(rowNoUsable), null);
  });

  // 2. isAllowedByCsp CSP 域名校验
  await t.test("2. isAllowedByCsp validates against exact CSP img-src hosts", () => {
    // data: 和 blob: 允许
    assert.equal(isAllowedByCsp("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="), true);
    assert.equal(isAllowedByCsp("blob:http://localhost:3000/1234"), true);

    // 允许的白名单 host
    assert.equal(
      isAllowedByCsp("https://nisar.asf.earthdatacloud.nasa.gov/previews/thumb.jpg"),
      true
    );
    assert.equal(
      isAllowedByCsp("https://planetarycomputer.microsoft.com/api/stac/v1/thumb.jpg"),
      true
    );
    assert.equal(isAllowedByCsp("https://tile.openstreetmap.org/1/2/3.png"), true);
    assert.equal(
      isAllowedByCsp("https://datapool.asf.alaska.edu/BROWSE/SB/S1A_IW_SLC.jpg"),
      true,
      "datapool.asf.alaska.edu 已在 CSP img-src 中放行，必须返回 true"
    );
    assert.equal(
      isAllowedByCsp("https://cumulus.asf.earthdatacloud.nasa.gov/browse.jpg"),
      true,
      "cumulus.asf.earthdatacloud.nasa.gov 已在 CSP img-src 中放行，必须返回 true"
    );
    assert.equal(
      isAllowedByCsp("https://bhoonidhi.nrsc.gov.in/browse/s_band.png"),
      true,
      "bhoonidhi.nrsc.gov.in 已在 CSP img-src 中放行，必须返回 true"
    );

    // 不在白名单的 host（将被 CSP 拦截，直接拒绝发起请求）
    assert.equal(
      isAllowedByCsp("https://example.invalid/browse.png"),
      false,
      "不在 CSP img-src 中的域名必须返回 false"
    );
    assert.equal(
      isAllowedByCsp("http://nisar.asf.earthdatacloud.nasa.gov/insecure.jpg"),
      false,
      "非 https 协议必须拒绝"
    );
  });

  // 3. createThumbnailQueue 纯逻辑并发、队列、缓存与不重试机制
  await t.test("3. createThumbnailQueue enforces max concurrency, FIFO, no retry, and cancel", async () => {
    let currentInFlight = 0;
    let peakInFlight = 0;
    const loadCalls: string[] = [];

    // 可控的延迟加载器
    const deferreds = new Map<
      string,
      { resolve: () => void; reject: (err: Error) => void }
    >();

    const mockLoad = (url: string) => {
      loadCalls.push(url);
      currentInFlight++;
      peakInFlight = Math.max(peakInFlight, currentInFlight);
      return new Promise<void>((resolve, reject) => {
        deferreds.set(url, {
          resolve: () => {
            currentInFlight--;
            resolve();
          },
          reject: (err: Error) => {
            currentInFlight--;
            reject(err);
          },
        });
      });
    };

    const queue = createThumbnailQueue({
      maxConcurrent: 4,
      load: mockLoad,
    });

    // 同时投递 6 个 URL
    const urls = [
      "https://host/1.jpg",
      "https://host/2.jpg",
      "https://host/3.jpg",
      "https://host/4.jpg",
      "https://host/5.jpg",
      "https://host/6.jpg",
    ];

    const promises = urls.map((u) => queue.enqueue(u));

    // 前 4 个应立即发起，后 2 个排队中
    assert.equal(queue.getInFlightCount(), 4);
    assert.equal(peakInFlight, 4);
    assert.deepEqual(loadCalls, [
      "https://host/1.jpg",
      "https://host/2.jpg",
      "https://host/3.jpg",
      "https://host/4.jpg",
    ]);

    // 此时取消还在排队中的第 6 个
    queue.cancel("https://host/6.jpg");

    // 完成第 1 个（成功）和第 2 个（失败）
    deferreds.get("https://host/1.jpg")!.resolve();
    deferreds.get("https://host/2.jpg")!.reject(new Error("Network 404"));

    // 等待 microtask 排队触发
    await Promise.resolve();
    await Promise.resolve();

    // 第 5 个应该被调度，第 6 个已被 cancel 不应发起
    assert.equal(loadCalls.includes("https://host/5.jpg"), true);
    assert.equal(loadCalls.includes("https://host/6.jpg"), false);

    // 完成第 3, 4, 5 个
    deferreds.get("https://host/3.jpg")!.resolve();
    deferreds.get("https://host/4.jpg")!.resolve();
    deferreds.get("https://host/5.jpg")!.resolve();

    const results = await Promise.all([
      promises[0],
      promises[1],
      promises[2],
      promises[3],
      promises[4],
    ]);

    assert.equal(results[0], "loaded");
    assert.equal(results[1], "failed");
    assert.equal(results[2], "loaded");
    assert.equal(results[3], "loaded");
    assert.equal(results[4], "loaded");

    // 验证失败不重试（No retry on failure）：
    // 再次 enqueue 之前失败的 URL 2，应立即解析为 failed 且不调用 load
    const beforeCallsCount = loadCalls.length;
    const retryResult = await queue.enqueue("https://host/2.jpg");
    assert.equal(retryResult, "failed");
    assert.equal(loadCalls.length, beforeCallsCount, "失败后绝不重新发起请求");

    // 验证成功缓存命中：
    // 再次 enqueue 之前成功的 URL 1，应立即解析为 loaded 且不调用 load
    const cachedResult = await queue.enqueue("https://host/1.jpg");
    assert.equal(cachedResult, "loaded");
    assert.equal(loadCalls.length, beforeCallsCount, "命中缓存不重复发起请求");
  });

  // 4. placeholderReason 区分无快视图与被 CSP 阻断
  await t.test("4. placeholderReason distinguishes between missing URL and blocked by CSP", () => {
    const missingRow: Partial<SceneRow> = { assets: [] };
    assert.equal(placeholderReason(missingRow), "none");

    const blockedRow: Partial<SceneRow> = {
      assets: [
        {
          role: "thumbnail",
          url: "https://example.invalid/browse.png",
        },
      ],
    };
    assert.equal(placeholderReason(blockedRow), "blocked");
  });

  // 5. 漂移测试：CSP_ALLOWED_IMAGE_HOSTS 必须与 src-tauri/tauri.conf.json 中 img-src 的 https 主机完全一致
  await t.test("5. CSP_ALLOWED_IMAGE_HOSTS matches tauri.conf.json img-src hosts exactly", () => {
    const __filename = fileURLToPath(import.meta.url);
    const tauriConfPath = path.resolve(path.dirname(__filename), "../../../../src-tauri/tauri.conf.json");
    const content = fs.readFileSync(tauriConfPath, "utf-8");
    const config = JSON.parse(content);
    const csp = config.app?.security?.csp;
    const imgSrc: string[] = Array.isArray(csp?.["img-src"])
      ? csp["img-src"]
      : typeof csp === "object" && csp !== null && Array.isArray(csp)
      ? csp
      : [];
    const expectedHosts = new Set<string>();
    for (const item of imgSrc) {
      if (typeof item === "string" && item.startsWith("https://")) {
        const parsed = new URL(item);
        expectedHosts.add(parsed.hostname);
      }
    }
    assert.deepStrictEqual(
      Array.from(CSP_ALLOWED_IMAGE_HOSTS).sort(),
      Array.from(expectedHosts).sort(),
      "CSP_ALLOWED_IMAGE_HOSTS 必须与 src-tauri/tauri.conf.json 的 img-src https 域名集合完全相等"
    );
  });
});
