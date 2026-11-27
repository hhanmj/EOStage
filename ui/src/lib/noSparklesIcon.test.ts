import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";

const UI_SRC_DIR = path.resolve(import.meta.dirname, "..");

function collectSourceFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectSourceFiles(full));
      continue;
    }
    if (!entry.isFile()) continue;
    if (!/\.(ts|tsx)$/.test(entry.name)) continue;
    if (/\.test\.(ts|tsx)$/.test(entry.name)) continue;
    if (entry.name === "geoscienceSkillsCatalog.ts") continue;
    files.push(full);
  }
  return files;
}

test("production UI avoids generic Sparkles icon and literal sparkle emoji", () => {
  const violations: string[] = [];
  for (const filePath of collectSourceFiles(UI_SRC_DIR)) {
    const content = readFileSync(filePath, "utf8");
    const rel = path.relative(UI_SRC_DIR, filePath);
    if (/\bSparkles\b/.test(content)) {
      violations.push(`${rel}: contains Lucide Sparkles reference`);
    }
    if (content.includes("✨") || content.includes("✦")) {
      violations.push(`${rel}: contains literal sparkle glyph`);
    }
  }
  assert.deepEqual(violations, []);
});
