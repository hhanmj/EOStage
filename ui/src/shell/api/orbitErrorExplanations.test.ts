import assert from "node:assert/strict";
import test from "node:test";

import {
  ERROR_CODE_MAP,
  explainErrorCode,
  formatErrorWithCode,
} from "./errorExplanations.ts";

test("ORB 轨道错误码解释（S1）：ORB001/ORB002/ORB003 不再显示裸码", async (suite) => {
  await suite.test("1. formatErrorWithCode 返回中文解释而非裸码", () => {
    for (const code of ["ORB001", "ORB002", "ORB003"] as const) {
      const text = formatErrorWithCode(code);
      assert.notStrictEqual(text, code);
      assert.match(text, /轨道/);
    }
  });

  await suite.test("2. explainErrorCode 返回非 null 解释", () => {
    for (const code of ["ORB001", "ORB002", "ORB003"] as const) {
      assert.notStrictEqual(explainErrorCode(code), null);
    }
  });

  await suite.test("3. ORB002 文案非恐吓：尚未发布 + 稍后重试", () => {
    assert.match(ERROR_CODE_MAP["ORB002"]!, /尚未发布/);
    assert.match(ERROR_CODE_MAP["ORB002"]!, /稍后重试/);
  });

  await suite.test("4. ORB003 文案说明等待上限与手动确认", () => {
    assert.match(ERROR_CODE_MAP["ORB003"]!, /上限/);
    assert.match(ERROR_CODE_MAP["ORB003"]!, /手动/);
  });

  await suite.test("5. ORB001 文案说明轨道服务失败", () => {
    assert.match(ERROR_CODE_MAP["ORB001"]!, /轨道/);
    assert.match(ERROR_CODE_MAP["ORB001"]!, /稍后重试/);
  });

  await suite.test("6. 未知码仍返回 null / 原样显示（守护）", () => {
    assert.strictEqual(explainErrorCode("ORB999"), null);
    assert.strictEqual(formatErrorWithCode("ORB999"), "ORB999");
    assert.strictEqual(explainErrorCode(null), null);
  });
});
