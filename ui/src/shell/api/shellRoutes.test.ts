import assert from "node:assert/strict";
import test from "node:test";
import {
  parseShellRoute,
  resolveInitialRoute,
  initialHashToWrite,
  routeMeta,
  readLastRoute,
  writeLastRoute,
  type SimpleStorage,
} from "./shellRoutes.ts";

test("shellRoutes - 路由解析与持久化套件", async (t) => {
  await t.test("parseShellRoute 正确识别所有路由及 plugins 路由", () => {
    assert.equal(parseShellRoute("#/agent"), "agent");
    assert.equal(parseShellRoute("#/history"), "history");
    assert.equal(parseShellRoute("#/workbench"), "workbench");
    assert.equal(parseShellRoute("#/downloads"), "downloads");
    assert.equal(parseShellRoute("#/plugins"), "plugins");
    assert.equal(parseShellRoute("#/settings"), "settings");

    // 未知或非法 hash 返回 null
    assert.equal(parseShellRoute("#/unknown"), null);
    assert.equal(parseShellRoute(""), null);
    assert.equal(parseShellRoute("#/plugins?query=1"), null);
  });

  await t.test("resolveInitialRoute 支持存储的 plugins 路由回退", () => {
    const memStorage: Record<string, string> = {
      "eostage.shell.lastRoute": "plugins",
    };
    const mockStorage: SimpleStorage = {
      getItem: (k) => memStorage[k] ?? null,
      setItem: (k, v) => {
        memStorage[k] = v;
      },
    };

    const stored = readLastRoute(mockStorage);
    assert.equal(stored, "plugins");
    assert.equal(resolveInitialRoute("", stored), "plugins");
    assert.equal(initialHashToWrite("", "plugins"), "#/plugins");
  });

  await t.test("routeMeta 返回三语支持的 plugins 标签", () => {
    const meta = routeMeta("plugins");
    assert.ok(meta.label && typeof meta.label === "string");
  });
});
