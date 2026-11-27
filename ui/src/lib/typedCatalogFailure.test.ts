import assert from "node:assert/strict";
import test from "node:test";

import { formatTypedCatalogFailure } from "./typedCatalogFailure.ts";

test("typed SAR failures keep the dataset, actionable reason, and safe code", () => {
  const message = formatTypedCatalogFailure("ALOS-2", {
    code: "CATALOGUE_RESULT_LIMIT_EXCEEDED",
    error: "application_service.unavailable",
  });

  assert.match(message, /^ALOS-2 检索失败/m);
  assert.match(message, /目录结果数量/);
  assert.match(message, /错误代码 CATALOGUE_RESULT_LIMIT_EXCEEDED/);
  assert.doesNotMatch(message, /application_service|操作失败，请稍后重试/);
});

test("unsafe error codes are not reflected", () => {
  const message = formatTypedCatalogFailure("ALOS-2", {
    code: "token=https://secret.invalid",
  });
  assert.doesNotMatch(message, /token|https:/i);
});
