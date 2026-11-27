import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as shellRoutes from "./shellRoutes.ts";
import type { ShellRoute } from "./shellRoutes.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

test("W5-6 R5 (B): initialHashToWrite 启动写入 hash 纯逻辑与接线测试", async (t) => {
  const fn = (shellRoutes as Record<string, unknown>).initialHashToWrite as
    | ((currentHash: string, route: ShellRoute) => string | null)
    | undefined;

  await t.test("1. 空 hash 按初始 route 返回 #/<route>", () => {
    assert.strictEqual(typeof fn, "function", "initialHashToWrite 必须导出");
    assert.strictEqual(fn!("", "agent"), "#/agent");
    assert.strictEqual(fn!("", "downloads"), "#/downloads");
  });

  await t.test("2. 已可解析的合法路由 hash 返回 null（已同步）", () => {
    assert.strictEqual(typeof fn, "function");
    assert.strictEqual(fn!("#/downloads", "downloads"), null);
    assert.strictEqual(fn!("#/agent", "agent"), null);
    assert.strictEqual(fn!("#/workbench", "workbench"), null);
  });

  await t.test("3. 含 settings 或 legacy 开发开关的 hash 返回 null（不触碰）", () => {
    assert.strictEqual(typeof fn, "function");
    assert.strictEqual(fn!("#settings=models", "agent"), null);
    assert.strictEqual(fn!("#legacy", "agent"), null);
  });

  await t.test("4. 其他无法解析的非开发 hash 返回 #/<route>", () => {
    assert.strictEqual(typeof fn, "function");
    assert.strictEqual(fn!("#garbage", "agent"), "#/agent");
  });

  await t.test("5. DesktopWorkbench.tsx 挂载时调用 initialHashToWrite 并写入 window.location.hash", () => {
    const wbPath = path.resolve(__dirname, "../../components/desktop/DesktopWorkbench.tsx");
    const src = fs.readFileSync(wbPath, "utf-8");
    assert.match(
      src,
      /initialHashToWrite\s*\(\s*window\.location\.hash\s*,\s*route\s*\)/,
      "DesktopWorkbench.tsx 必须调用 initialHashToWrite(window.location.hash, route)"
    );
    assert.match(
      src,
      /if\s*\(\s*h\s*\)\s*\{\s*window\.location\.hash\s*=\s*h\s*;?\s*\}|if\s*\(\s*h\s*\)\s*window\.location\.hash\s*=\s*h/,
      "DesktopWorkbench.tsx 必须在 h 非空时赋值给 window.location.hash"
    );
  });
});
