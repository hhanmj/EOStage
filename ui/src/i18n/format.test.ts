import assert from "node:assert/strict";
import test from "node:test";
import { translate } from "./index.ts";
import { formatArea, formatBytes, formatCompactNumber, formatDate, formatNumber, pluralRule } from "./format.ts";

test("translation interpolates values, selects English plurals, and falls back to Chinese", () => {
  assert.equal(translate("en", "common.sample.greeting", { name: "Lin" }), "Hello, Lin");
  assert.equal(translate("en", "common.sample.count", { count: 1 }), "1 item");
  assert.equal(translate("en", "common.sample.count", { count: 2 }), "2 items");
  assert.equal(translate("ja", "common.sample.count", { count: 2 }), "2 件");
  assert.equal(translate("en", "agent.conversation.scenes", { count: 12 }), "12 scenes");
  assert.equal(translate("ja", "agent.conversation.scenes", { count: 12 }), "12 シーン");
  assert.equal(translate("en", "settings.language.missing" as never), "settings.language.missing");
});

test("Intl formatting preserves decimal SI byte units and localizes only digits", () => {
  assert.equal(formatBytes(29_070_721_024, "zh-CN"), "29.1 GB");
  assert.equal(formatBytes(1_500_000_000, "en"), "1.5 GB");
  assert.equal(formatBytes(1234, "ja"), "1.2 KB");
  assert.equal(formatBytes(null, "en"), "—");
  assert.equal(formatBytes(1234567, "en"), "1.2 MB");
  assert.equal(formatNumber(123456.5, "en"), "123,456.5");
  assert.equal(formatCompactNumber(510000000, "en"), "510M");
  assert.equal(formatArea(1234.5, "en"), "1,234.5 km²");
  assert.equal(formatDate(Date.UTC(2026, 10, 2), "en", { timeZone: "UTC", dateStyle: "medium" }), "Nov 2, 2026");
  assert.equal(pluralRule(1, "en"), "one");
  assert.equal(pluralRule(1, "ja"), "other");
});
