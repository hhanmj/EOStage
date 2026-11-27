#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;

function location(source, offset) {
  const before = source.slice(0, offset);
  const lines = before.split("\n");
  return { line: lines.length, column: lines.at(-1).length + 1 };
}

export function findResidualCjk(source) {
  const findings = [];
  const lines = source.split(/\r?\n/);
  const exemptLines = new Set();
  lines.forEach((line, index) => {
    const marker = line.match(/\/\/ i18n-exempt:(.*)$/);
    if (marker && marker[1].trim()) exemptLines.add(index + 1);
  });
  let state = "code";
  let quote = "";
  let escaped = false;

  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (state === "line-comment") {
      if (character === "\n") state = "code";
      continue;
    }
    if (state === "block-comment") {
      if (character === "*" && next === "/") {
        state = "code";
        index += 1;
      }
      continue;
    }
    if (state === "string" || state === "template") {
      if (escaped) {
        escaped = false;
      } else if (character === "\\") {
        escaped = true;
      } else if (character === quote) {
        state = "code";
      } else if (CJK.test(character)) {
        const start = index;
        while (index + 1 < source.length && CJK.test(source[index + 1])) index += 1;
        const finding = { ...location(source, start), text: source.slice(start, index + 1) };
        if (!exemptLines.has(finding.line)) findings.push(finding);
      }
      continue;
    }
    if (character === "/" && next === "/") {
      state = "line-comment";
      index += 1;
    } else if (character === "/" && next === "*") {
      state = "block-comment";
      index += 1;
    } else if (character === '"' || character === "'") {
      state = "string";
      quote = character;
    } else if (character === "`") {
      state = "template";
      quote = character;
    } else if (CJK.test(character) && source.slice(0, index).trimEnd().endsWith(">")) {
      const finding = { ...location(source, index), text: character };
      if (!exemptLines.has(finding.line)) findings.push(finding);
    }
  }
  return findings;
}

export function checkFiles(files) {
  const failures = [];
  for (const file of files) {
    let source;
    try {
      source = fs.readFileSync(file, "utf8");
    } catch (error) {
      failures.push(`${file}: cannot read file (${error.message})`);
      continue;
    }
    for (const finding of findResidualCjk(source)) {
      failures.push(`${file}:${finding.line}:${finding.column}: hard-coded CJK text (${finding.text})`);
    }
  }
  return failures;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const files = process.argv.slice(2);
  if (files.length === 0) {
    console.error("Usage: node scripts/check_i18n_residual_cjk.mjs <file> [...]");
    process.exit(2);
  }
  const failures = checkFiles(files);
  if (failures.length > 0) {
    console.error(failures.join("\n"));
    process.exit(1);
  }
  console.log(`No residual CJK string literals in ${files.length} file(s).`);
}
