import assert from "node:assert/strict";
import test from "node:test";
import { MESSAGES } from "./locales.ts";
import type { Locale } from "./types.ts";

function flatten(value: unknown, prefix = ""): Map<string, string> {
  const entries = new Map<string, string>();
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof child === "string") entries.set(path, child);
    else {
      for (const [nested, text] of flatten(child, path)) entries.set(nested, text);
    }
  }
  return entries;
}

function placeholders(template: string): string[] {
  return [...template.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1]).sort();
}

for (const locale of ["en", "ja"] as const satisfies readonly Locale[]) {
  test(`${locale} has every non-empty zh-CN message with matching placeholders`, () => {
    const translated = flatten(MESSAGES[locale]);
    for (const [key, source] of flatten(MESSAGES["zh-CN"])) {
      const message = translated.get(key);
      assert.equal(typeof message, "string", `${locale} is missing ${key}`);
      assert.ok(message.trim(), `${locale} has an empty message at ${key}`);
      assert.deepEqual(placeholders(message), placeholders(source), `${locale} placeholders differ at ${key}`);
    }
  });
}
