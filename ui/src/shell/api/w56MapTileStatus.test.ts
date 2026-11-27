import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import * as tileStatusModule from "./mapTileStatus.ts";
import {
  initialBaseTileRuntime,
  baseTileOnTileload,
  baseTileOnTileerror,
  baseTileOnLoadEvent,
} from "./mapTileStatus.ts";

const here = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R4 底图瓦片状态机与 WorkbenchMap 源码接线", async (t) => {
  await t.test("1. 模块仅导出四个规定函数", () => {
    assert.deepEqual(
      Object.keys(tileStatusModule).sort(),
      [
        "baseTileOnLoadEvent",
        "baseTileOnTileerror",
        "baseTileOnTileload",
        "initialBaseTileRuntime",
      ],
      "mapTileStatus.ts 应仅导出四个规定函数",
    );
  });

  await t.test("2. initialBaseTileRuntime 与 tileload -> ready", () => {
    const init = initialBaseTileRuntime();
    assert.deepEqual(init, { status: "loading", loadedCount: 0, errorCount: 0 });

    const afterOneLoad = baseTileOnTileload(init);
    assert.deepEqual(
      afterOneLoad,
      { status: "ready", loadedCount: 1, errorCount: 0 },
      "tileload 后 loadedCount 应 +1 且 status 置为 ready",
    );
  });

  await t.test("3. 首错且 0 载 -> failed；错后 tileload -> ready；有载后再错保持 ready", () => {
    const init = initialBaseTileRuntime();
    const afterFirstErr = baseTileOnTileerror(init);
    assert.deepEqual(
      afterFirstErr,
      { status: "failed", loadedCount: 0, errorCount: 1 },
      "loadedCount 为 0 时发生 tileerror 应置为 failed",
    );

    const recovered = baseTileOnTileload(afterFirstErr);
    assert.deepEqual(
      recovered,
      { status: "ready", loadedCount: 1, errorCount: 1 },
      "失败后若有瓦片成功 tileload 应恢复为 ready",
    );

    const errAfterReady = baseTileOnTileerror(recovered);
    assert.deepEqual(
      errAfterReady,
      { status: "ready", loadedCount: 1, errorCount: 2 },
      "loadedCount > 0 时再次发生 tileerror 应保持 ready",
    );
  });

  await t.test("4. loadEvent：0 载时保持原 status（零瓦片不得就绪），有载时置为 ready", () => {
    const init = initialBaseTileRuntime();
    const loadWithZeroTiles = baseTileOnLoadEvent(init);
    assert.deepEqual(
      loadWithZeroTiles,
      { status: "loading", loadedCount: 0, errorCount: 0 },
      "loadedCount 为 0 时触发 load 事件必须保持 loading，不得误报 ready",
    );

    const failedZero = baseTileOnTileerror(init);
    const loadAfterFailedZero = baseTileOnLoadEvent(failedZero);
    assert.deepEqual(
      loadAfterFailedZero,
      { status: "failed", loadedCount: 0, errorCount: 1 },
      "loadedCount 为 0 且已 failed 时触发 load 事件应保持 failed",
    );

    const loaded = baseTileOnTileload(init);
    const loadWithTiles = baseTileOnLoadEvent({ ...loaded, status: "loading" });
    assert.deepEqual(
      loadWithTiles,
      { status: "ready", loadedCount: 1, errorCount: 0 },
      "loadedCount > 0 时触发 load 事件应置为 ready",
    );
  });

  await t.test("5. 重置：再次调用 initialBaseTileRuntime 返回全新 loading 初始态", () => {
    const dirty = baseTileOnTileload(baseTileOnTileerror(initialBaseTileRuntime()));
    assert.equal(dirty.status, "ready");
    const reset = initialBaseTileRuntime();
    assert.deepEqual(reset, { status: "loading", loadedCount: 0, errorCount: 0 });
  });

  await t.test("6. WorkbenchMap.tsx 基础瓦片层不再直接 load: () => setMapStatus(\"external_tiles_ready\")", () => {
    const mapPath = path.resolve(here, "../../components/WorkbenchMap.tsx");
    const src = fs.readFileSync(mapPath, "utf-8");

    assert.ok(
      !src.includes('load: () => setMapStatus("external_tiles_ready")'),
      'WorkbenchMap.tsx 基础层不得再直接写 load: () => setMapStatus("external_tiles_ready")',
    );
    assert.ok(
      src.includes("baseTileOnLoadEvent") &&
        src.includes("baseTileOnTileload") &&
        src.includes("baseTileOnTileerror"),
      "WorkbenchMap.tsx 必须接入 baseTileOnLoadEvent / baseTileOnTileload / baseTileOnTileerror",
    );
  });
});
