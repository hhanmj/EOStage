import assert from "node:assert/strict";
import { MESSAGES } from "../../i18n/locales.ts";

export function assertLocalizedSource(source: string, key: string, literal: string): void {
  assert.ok(source.includes(key), `source must use the i18n key ${key}`);
  let value: unknown = MESSAGES["zh-CN"];
  for (const part of key.split(".")) {
    assert.ok(value && typeof value === "object", `zh-CN dictionary must contain ${key}`);
    value = (value as Record<string, unknown>)[part];
  }
  assert.equal(value, literal, `zh-CN value for ${key} must remain byte-identical`);
}
