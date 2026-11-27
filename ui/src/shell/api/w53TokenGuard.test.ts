import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const uiSrcDir = path.resolve(__dirname, "../..");

function collectFiles(dir: string, recursive: boolean = true): string[] {
  const result: string[] = [];
  if (!fs.existsSync(dir)) return result;
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory() && recursive) {
      result.push(...collectFiles(fullPath, recursive));
    } else if (entry.isFile() && fullPath.endsWith(".tsx")) {
      result.push(fullPath);
    }
  }
  return result;
}

test("W5-3b Token Guard: 颜色工具类必须使用 ui/src/index.css @theme inline 中合法定义的 --color-sp-* 令牌", () => {
  const indexCssPath = path.resolve(uiSrcDir, "index.css");
  const indexCss = fs.readFileSync(indexCssPath, "utf8");

  // 1. 提取 @theme inline 内所有 --color-sp-<name>
  const themeMatch = indexCss.match(/@theme inline\s*\{([^}]*)\}/);
  assert.ok(themeMatch, "ui/src/index.css must contain @theme inline block");

  const validColorNames = new Set<string>();
  const colorTokenRegex = /--color-sp-([\w-]+)\s*:/g;
  let match: RegExpExecArray | null;
  while ((match = colorTokenRegex.exec(themeMatch[1])) !== null) {
    validColorNames.add(match[1]);
  }
  assert.ok(validColorNames.size > 0, "Must have valid --color-sp-* tokens defined");

  // 2. 扫描 ui/src/components/desktop/*.tsx 与 ui/src/shell/components/**/*.tsx
  const desktopFiles = collectFiles(path.resolve(uiSrcDir, "components/desktop"), false);
  const shellComponentFiles = collectFiles(path.resolve(uiSrcDir, "shell/components"), true);
  const targetFiles = [...desktopFiles, ...shellComponentFiles];

  const colorPrefixes =
    "(?:text|bg|ring|border|fill|stroke|outline|divide|placeholder|from|to|via|decoration|accent|caret)";
  const pattern = new RegExp(`(?<![\\w-])${colorPrefixes}-sp-([\\w-]+)`, "g");

  const violations: Array<{ file: string; line: number; token: string; base: string }> = [];

  for (const filePath of targetFiles) {
    const content = fs.readFileSync(filePath, "utf8");
    const lines = content.split(/\r?\n/);

    for (let i = 0; i < lines.length; i++) {
      const lineText = lines[i];
      let m: RegExpExecArray | null;
      pattern.lastIndex = 0;
      while ((m = pattern.exec(lineText)) !== null) {
        const fullToken = m[0];
        const rawName = m[1];
        // 去除透明度修饰 如 /80 或 /50
        const baseName = rawName.split("/")[0];

        if (!validColorNames.has(baseName)) {
          violations.push({
            file: path.relative(uiSrcDir, filePath).replace(/\\/g, "/"),
            line: i + 1,
            token: fullToken,
            base: baseName,
          });
        }
      }
    }
  }

  assert.strictEqual(
    violations.length,
    0,
    `Found invalid sp color tokens in components:\n${violations
      .map((v) => `  ${v.file}:${v.line} -> ${v.token} (unknown color '--color-sp-${v.base}')`)
      .join("\n")}`
  );
});
