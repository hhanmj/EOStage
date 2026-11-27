import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const indexCssPath = path.resolve(__dirname, "../../index.css");

export interface ParsedCssRule {
  selector: string;
  body: string;
  inLayer: boolean;
  layerName?: string;
}

/**
 * 按花括号深度扫描 CSS 规则并跟踪 @layer 上下文。
 */
export function parseCssRules(cssText: string): ParsedCssRule[] {
  // 1. 去除所有 /* ... */ 注释
  const stripped = cssText.replace(/\/\*[\s\S]*?\*\//g, "");

  const rules: ParsedCssRule[] = [];
  interface StackEntry {
    isLayer: boolean;
    layerName?: string;
    isRule: boolean;
    header: string;
    body: string;
  }
  const stack: StackEntry[] = [];
  let header = "";

  for (let i = 0; i < stripped.length; i++) {
    const char = stripped[i];

    if (char === "{") {
      const trimmedHeader = header.trim();
      const layerMatch = trimmedHeader.match(/^@layer\s+([\w-]+)/);
      const isLayer = !!layerMatch;
      const layerName = layerMatch ? layerMatch[1] : undefined;

      // 容器指令（如 @layer, @media, @supports, @container, @keyframes）不是样式规则叶节点
      const isContainer =
        trimmedHeader.startsWith("@media") ||
        trimmedHeader.startsWith("@supports") ||
        trimmedHeader.startsWith("@layer") ||
        trimmedHeader.startsWith("@container") ||
        trimmedHeader.startsWith("@keyframes");
      const isRule = !isContainer;

      const inLayer = stack.some((entry) => entry.isLayer) || isLayer;
      const activeLayer = stack.find((entry) => entry.isLayer)?.layerName || layerName;

      stack.push({
        isLayer: inLayer,
        layerName: activeLayer,
        isRule,
        header: trimmedHeader,
        body: "",
      });
      header = "";
    } else if (char === "}") {
      const popped = stack.pop();
      if (popped && popped.isRule) {
        rules.push({
          selector: popped.header,
          body: popped.body.trim(),
          inLayer: popped.isLayer,
          layerName: popped.layerName,
        });
      }
      header = "";
    } else if (char === ";" && (stack.length === 0 || !stack[stack.length - 1].isRule)) {
      // 顶层或容器层语句结束（如 @import ...;）
      header = "";
    } else {
      if (stack.length > 0 && stack[stack.length - 1].isRule) {
        stack[stack.length - 1].body += char;
      } else {
        header += char;
      }
    }
  }

  return rules;
}

test("W5-2g 2g-6: 主题边框颜色分层与变量守卫", async (t) => {
  const indexCss = fs.readFileSync(indexCssPath, "utf8");
  const rules = parseCssRules(indexCss);

  await t.test(
    "1. 不存在未分层（不在任何 @layer 块内）的、选择器恰为 * 的 border-color 规则",
    () => {
      const unlayeredUniversalBorderRules = rules.filter(
        (r) =>
          !r.inLayer &&
          r.selector.replace(/\s+/g, " ").trim() === "*" &&
          /border-color\s*:\s*var\(--color-border\)/.test(r.body),
      );
      assert.equal(
        unlayeredUniversalBorderRules.length,
        0,
        `不应存在未分层的 * { border-color } 规则，但发现了 ${unlayeredUniversalBorderRules.length} 处`,
      );
    },
  );

  await t.test(
    "2. 存在 @layer base { * { border-color: var(--color-border); } }（允许空白差异）",
    () => {
      const layeredBaseUniversalBorderRules = rules.filter(
        (r) =>
          r.inLayer &&
          r.layerName === "base" &&
          r.selector.replace(/\s+/g, " ").trim() === "*" &&
          /border-color\s*:\s*var\(--color-border\)/.test(r.body),
      );
      assert.ok(
        layeredBaseUniversalBorderRules.length > 0,
        "必须存在 @layer base { * { border-color: var(--color-border); } } 规则",
      );
      assert.match(
        indexCss,
        /@layer\s+base\s*\{\s*\*\s*\{\s*border-color\s*:\s*var\(--color-border\)\s*;\s*\}\s*\}/,
        "index.css 中必须包含 @layer base { * { border-color: var(--color-border); } }",
      );
    },
  );

  await t.test(
    "3. 不存在未分层的 .dark * border 规则且存在 @layer base 内的 .dark * { border-color: var(--color-border); }",
    () => {
      const unlayeredDarkBorderRules = rules.filter(
        (r) =>
          !r.inLayer &&
          r.selector.replace(/\s+/g, " ").trim() === ".dark *" &&
          /border-color\s*:\s*var\(--color-border\)/.test(r.body),
      );
      assert.equal(
        unlayeredDarkBorderRules.length,
        0,
        `不应存在未分层的 .dark * { border-color } 规则，但发现了 ${unlayeredDarkBorderRules.length} 处`,
      );

      const layeredBaseDarkBorderRules = rules.filter(
        (r) =>
          r.inLayer &&
          r.layerName === "base" &&
          r.selector.replace(/\s+/g, " ").trim() === ".dark *" &&
          /border-color\s*:\s*var\(--color-border\)/.test(r.body),
      );
      assert.ok(
        layeredBaseDarkBorderRules.length > 0,
        "必须存在 @layer base { .dark * { border-color: var(--color-border); } } 规则",
      );
      assert.match(
        indexCss,
        /@layer\s+base\s*\{\s*\.dark\s+\*\s*\{\s*border-color\s*:\s*var\(--color-border\)\s*;\s*\}\s*\}/,
        "index.css 中必须包含 @layer base { .dark * { border-color: var(--color-border); } }",
      );
    },
  );

  await t.test(
    "4. 主题变量：浅色与深色 --sp-line 均为 0.14（指挥端 2026-10-27 决定），--border 未改动",
    () => {
      const rootMatch = indexCss.match(/:root\s*\{([^}]*)\}/);
      assert.ok(rootMatch, "index.css 必须包含 :root 块");
      assert.match(
        rootMatch[1],
        /--sp-line\s*:\s*rgba\(\s*0\s*,\s*0\s*,\s*0\s*,\s*0\.14\s*\)/,
        "浅色 --sp-line 按指挥端 2026-10-27 决定为 rgba(0, 0, 0, 0.14)",
      );
      assert.match(
        rootMatch[1],
        /--border\s*:\s*rgba\(\s*255\s*,\s*255\s*,\s*255\s*,\s*0\.58\s*\)/,
        "浅色 :root 必须保持 --border: rgba(255, 255, 255, 0.58) 未被改动",
      );

      const darkMatch = indexCss.match(/\.dark\s*\{([^}]*)\}/);
      assert.ok(darkMatch, "index.css 必须包含 .dark 块");
      assert.match(
        darkMatch[1],
        /--sp-line\s*:\s*rgba\(\s*255\s*,\s*255\s*,\s*255\s*,\s*0\.14\s*\)/,
        "深色 .dark 按指挥端 2026-10-27 决定提升为 rgba(255, 255, 255, 0.14)",
      );
      assert.match(
        darkMatch[1],
        /--border\s*:\s*rgba\(\s*255\s*,\s*255\s*,\s*255\s*,\s*0\.14\s*\)/,
        "深色 .dark 必须保持 --border: rgba(255, 255, 255, 0.14) 未被改动",
      );
    },
  );
});
