import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const UI_SRC_ROOT = path.resolve(__dirname, "..");

function collectTsxFiles(dirPath: string): string[] {
  const results: string[] = [];
  if (!fs.existsSync(dirPath)) return results;
  for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
    const fullPath = path.join(dirPath, entry.name);
    if (entry.isDirectory()) {
      results.push(...collectTsxFiles(fullPath));
    } else if (entry.isFile() && fullPath.endsWith(".tsx") && !fullPath.endsWith(".test.tsx")) {
      results.push(fullPath);
    }
  }
  return results;
}

function stripCommentsPreservingLines(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\r\n]/g, " "))
    .replace(/\/\/.*$/gm, "");
}

function findVisibleNativeSelectViolations(filePath: string): Array<{ line: number; snippet: string }> {
  const raw = stripCommentsPreservingLines(fs.readFileSync(filePath, "utf8"));
  const lines = raw.split(/\r?\n/);
  const violations: Array<{ line: number; snippet: string }> = [];

  for (let i = 0; i < lines.length; i += 1) {
    const match = lines[i].match(/<select(?:\s|>|$)/);
    if (!match) continue;
    // Scan from '<select' across lines tracking brace depth so '=>' inside props does not terminate the tag early.
    const startOffset = match.index ?? 0;
    let tagChunk = lines[i].slice(startOffset);
    let j = i;
    let braceDepth = 0;
    let openTagComplete = false;
    let openTag = tagChunk;

    while (!openTagComplete && j < lines.length && j - i < 30) {
      for (let k = j === i ? "<select".length : tagChunk.length - lines[j].length; k < tagChunk.length; k += 1) {
        const ch = tagChunk[k];
        if (ch === "{") braceDepth += 1;
        else if (ch === "}" && braceDepth > 0) braceDepth -= 1;
        else if (ch === ">" && braceDepth === 0) {
          openTag = tagChunk.slice(0, k + 1);
          openTagComplete = true;
          break;
        }
      }
      if (openTagComplete) break;
      j += 1;
      if (j < lines.length) {
        tagChunk += "\n" + lines[j];
      }
    }

    // Allow explicit sr-only accessibility/compat proxies only.
    if (/className\s*=\s*["'][^"']*\bsr-only\b[^"']*["']/.test(openTag)) {
      continue;
    }
    violations.push({ line: i + 1, snippet: lines[i].trim() });
  }
  return violations;
}

test("DESIGN.md 强制规则：现役桌面端与地图组件严禁使用可见原生 <select>（统一使用 <SpSelect>）", () => {
  const targetFiles = [
    ...collectTsxFiles(path.join(UI_SRC_ROOT, "components/desktop")),
    ...collectTsxFiles(path.join(UI_SRC_ROOT, "shell")),
    path.join(UI_SRC_ROOT, "components/WorkbenchMap.tsx"),
    path.join(UI_SRC_ROOT, "components/AoiDrawMap.tsx"),
  ];

  assert.ok(targetFiles.length > 20, "Should scan desktop/shell/map TSX components");

  const failures: string[] = [];
  for (const absPath of targetFiles) {
    const rel = path.relative(UI_SRC_ROOT, absPath);
    for (const v of findVisibleNativeSelectViolations(absPath)) {
      failures.push(`${rel}:${v.line}: ${v.snippet}`);
    }
  }

  assert.deepEqual(
    failures,
    [],
    `Found visible raw <select> elements that violate DESIGN.md (must migrate to <SpSelect>):\n${failures.join("\n")}`,
  );
});
