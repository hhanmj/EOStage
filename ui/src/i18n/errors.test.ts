import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { formatBridgeErrorValue } from "../lib/bridgeErrorFormat.ts";
import {
  ERROR_CODE_MAP,
  explainActionError,
  explainErrorCode,
  explainMessageKey,
  formatErrorWithCode,
  lookupErrorText,
  preferCodeExplanation,
} from "../shell/api/errorExplanations.ts";
import { MESSAGES } from "./locales.ts";

// Outputs of the pre-i18n literal tables, captured before they moved into the errors namespace.
type ParityFixture = {
  generic: string;
  explanations: Record<string, string>;
  bridge: Record<string, string>;
  messages: Array<string | null>;
  rows: Record<string, string[]>;
  bridgeCases: Array<{ input: unknown; context?: { dataset?: string }; output: string }>;
  explanationCases: Array<{ code: string; explain: string | null; format: string; action: string }>;
};
const fixture = JSON.parse(
  readFileSync(new URL("./errorsZhParity.fixture.json", import.meta.url), "utf8"),
) as ParityFixture;

const zh = MESSAGES["zh-CN"].errors;
const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
const KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;

test("zh-CN error tables are byte-identical to the previous literal tables", () => {
  assert.deepEqual(zh.explanations, fixture.explanations);
  assert.deepEqual(ERROR_CODE_MAP, fixture.explanations);
  const { ASF_QUERY_INVALID, ...bridge } = zh.bridge;
  assert.deepEqual(bridge, fixture.bridge);
  assert.equal(ASF_QUERY_INVALID, "当前 SAR 检索条件无效，请检查当前数据集的筛选条件。");
  assert.equal(Object.keys(zh.explanations).length, 30);
  assert.equal(Object.keys(zh.bridge).length, 258);
});

test("zh-CN explainErrorCode, formatErrorWithCode and explainActionError match previous output for every code", () => {
  for (const item of fixture.explanationCases) {
    assert.equal(explainErrorCode(item.code), item.explain, `explainErrorCode(${item.code})`);
    assert.equal(formatErrorWithCode(item.code), item.format, `formatErrorWithCode(${item.code})`);
    assert.equal(explainActionError({ code: item.code }, "FALLBACK"), item.action, `explainActionError(${item.code})`);
  }
});

test("zh-CN formatBridgeErrorValue matches previous output for every code and message shape", () => {
  for (const [code, outputs] of Object.entries(fixture.rows)) {
    fixture.messages.forEach((error, index) => {
      const expected = outputs[index] === "@friendly"
        ? fixture.bridge[code]
        : outputs[index] === "@message"
          ? error
          : outputs[index] === "@generic"
            ? fixture.generic
            : outputs[index];
      const input = error === null ? { code } : { code, error };
      assert.equal(formatBridgeErrorValue(input), expected, `${code} with ${JSON.stringify(error)}`);
    });
  }
  for (const item of fixture.bridgeCases) {
    // Error instances serialize to {}; their branches are asserted separately below.
    if (item.input && typeof item.input === "object" && Object.keys(item.input).length === 0) continue;
    assert.equal(formatBridgeErrorValue(item.input, item.context), item.output, JSON.stringify(item));
  }
});

test("zh-CN parity covers the special branches and Error inputs", () => {
  assert.equal(formatBridgeErrorValue(new Error("NoneType object has no attribute model_dump")), "当前结果缺少场景覆盖范围，请重新检索后重试。");
  assert.equal(formatBridgeErrorValue(new Error("model_dump failed")), "数据读取失败，请重新选择当前项目或研究区后重试。");
  assert.equal(formatBridgeErrorValue(new Error("something else")), fixture.generic);
  assert.equal(formatBridgeErrorValue({ code: "ASF_QUERY_INVALID" }, { dataset: "SLC-BURST" }), "当前 Burst 检索条件无效，请检查时间、AOI 和筛选参数后重试。");
});

test("en and ja carry every error code with non-empty, translated text", () => {
  for (const locale of ["en", "ja"] as const) {
    const errors = MESSAGES[locale].errors;
    for (const table of ["explanations", "bridge", "bridgeText"] as const) {
      const source = zh[table] as Record<string, string>;
      const translated = errors[table] as Record<string, string>;
      assert.deepEqual(Object.keys(translated).sort(), Object.keys(source).sort(), `${locale}.${table} keys`);
      for (const [code, text] of Object.entries(source)) {
        const value = translated[code];
        assert.ok(value?.trim(), `${locale}.${table}.${code} is empty`);
        assert.notEqual(value, text, `${locale}.${table}.${code} is untranslated`);
        if (locale === "en") assert.doesNotMatch(value, CJK, `en.${table}.${code} contains CJK`);
        if (locale === "ja" && CJK.test(text)) assert.match(value, KANA, `ja.${table}.${code} is not Japanese`);
      }
    }
    for (const [group, entries] of Object.entries(zh.messageKeys)) {
      for (const name of Object.keys(entries)) {
        const value = explainMessageKey(`${group}.${name}`);
        assert.ok(value);
        const localized = (errors.messageKeys as Record<string, Record<string, string>>)[group]?.[name];
        assert.ok(localized?.trim(), `${locale}.messageKeys.${group}.${name}`);
      }
    }
  }
});

test("lookupErrorText resolves each locale and never invents an explanation", () => {
  assert.equal(lookupErrorText("explanations", "DISK_LOW", "en"), MESSAGES.en.errors.explanations.DISK_LOW);
  assert.equal(lookupErrorText("bridge", "ASF_TIMEOUT", "ja"), MESSAGES.ja.errors.bridge.ASF_TIMEOUT);
  assert.notEqual(lookupErrorText("bridge", "ORB001", "en"), lookupErrorText("explanations", "ORB001", "en"));
  for (const locale of ["zh-CN", "en", "ja"] as const) {
    for (const code of ["UNKNOWN_CODE_X", "", "toString", "constructor", "__proto__"]) {
      assert.equal(lookupErrorText("explanations", code, locale), null, `${locale} ${code}`);
      assert.equal(lookupErrorText("bridge", code, locale), null, `${locale} ${code}`);
    }
    assert.equal(lookupErrorText("explanations", null, locale), null);
    assert.equal(lookupErrorText("explanations", 42, locale), null);
  }
  assert.equal(formatErrorWithCode("UNKNOWN_CODE_X"), "UNKNOWN_CODE_X");
  assert.equal(explainErrorCode("UNKNOWN_CODE_X"), null);
});

test("shell message_key maps to its zh-CN text; unknown keys stay unmapped", () => {
  assert.equal(explainMessageKey("application_service.unavailable"), fixture.bridge.V3_APPLICATION_UNAVAILABLE);
  assert.equal(explainMessageKey("application_service.invalid_request"), fixture.bridge.V3_APPLICATION_INVALID_REQUEST);
  assert.equal(explainMessageKey("application_service.gacos_task_failed"), fixture.bridge.GACOS_TASK_FAILED);
  assert.equal(explainMessageKey("shell.origin_not_allowed"), "当前页面来源未获准调用桌面功能，请重新打开 EOStage 窗口。");
  assert.equal(explainMessageKey("compat_worker.operation_failed"), "兼容组件操作失败，请重试；若持续出现，请修复或重新安装 EOStage 组件。");
  for (const key of ["fixture-sensitive", "disk_low", "shell.unknown", "shell.origin_not_allowed.extra", "toString.call", "", "ASF 认证不可用"]) {
    assert.equal(explainMessageKey(key), null, key);
  }
  assert.equal(explainMessageKey(undefined), null);
});

test("a projected message_key renders exactly like the raw key it replaces", () => {
  for (const [group, entries] of Object.entries(zh.messageKeys)) {
    for (const [name, text] of Object.entries(entries)) {
      for (const code of ["CATALOGUE_CAPABILITY_UNAVAILABLE", "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE", "UNKNOWN_CODE_X", "V3_APPLICATION_UNAVAILABLE"]) {
        assert.equal(
          formatBridgeErrorValue({ code, error: text }),
          formatBridgeErrorValue({ code, error: `${group}.${name}` }),
          `${code} with ${group}.${name}`,
        );
      }
    }
  }
});

test("zh-CN keeps the backend message_zh ahead of the code explanation", () => {
  assert.equal(preferCodeExplanation("后端原因", "DISK_LOW"), "后端原因");
  assert.equal(preferCodeExplanation("", "DISK_LOW"), fixture.explanations.DISK_LOW);
  assert.equal(preferCodeExplanation(undefined, "UNKNOWN_CODE_X"), null);
  assert.equal(formatBridgeErrorValue({ code: "ASF_TIMEOUT", error: "后端返回的中文说明" }), "后端返回的中文说明");
});
