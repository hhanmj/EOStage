import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { formatBridgeErrorValue as formatError } from "./bridgeErrorFormat.ts";

const authCases = [
  ["BHOONIDHI_CREDENTIALS_MISSING", /未配置.*凭据/],
  ["BHOONIDHI_CREDENTIALS_INVALID", /凭据.*不完整|凭据.*无效/],
  ["BHOONIDHI_AUTH_REJECTED", /拒绝登录|拒绝/],
  ["BHOONIDHI_ACTIVATION_REQUIRED", /激活/],
  ["BHOONIDHI_MAX_SESSIONS", /会话.*上限/],
  ["BHOONIDHI_AUTH_TRANSPORT_FAILED", /无法连接|网络|代理/],
  ["BHOONIDHI_AUTH_TIMEOUT", /超时/],
  ["BHOONIDHI_AUTH_RESPONSE_INVALID", /响应异常/],
  ["BHOONIDHI_REFRESH_UNAVAILABLE", /无法刷新/],
  ["BHOONIDHI_REFRESH_EXHAUSTED", /刷新失败/],
  ["BHOONIDHI_SESSION_CHANGED", /会话.*变化/],
  ["BHOONIDHI_SESSION_UNAVAILABLE", /无可用会话/],
  ["BHOONIDHI_SYNCHRONIZATION_FAILED", /同步失败/],
  ["BHOONIDHI_OAUTH_CONFIGURATION_INVALID", /认证配置/],
] as const;

test("Bhoonidhi copy is keyed to actual native categories, not an inferred HTTP cause", () => {
  const source = readFileSync(new URL("../../../crates/insar-download/src/bhoonidhi_oauth.rs", import.meta.url), "utf8");
  for (const [code, expected] of authCases) {
    assert.ok(source.includes(`\"${code}\"`), `${code} must exist in native BhoonidhiAuthError::code`);
    const message = formatError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /Sentinel-2|CDSE|Landsat|USGS|操作失败，请稍后重试/, code);
  }
});

test("unclassified Bhoonidhi auth stays concise without inventing a password cause", () => {
  const message = formatError({ code: "BHOONIDHI_AUTH_UNAVAILABLE", error: "Bhoonidhi 认证不可用" });
  assert.equal(message, "Bhoonidhi 未登录，请刷新状态或重新登录。");
  assert.doesNotMatch(message, /密码|不能证明|内部/);
});

test("safe specific auth context survives but credential-bearing context is not echoed", () => {
  const message = formatError({ code: "BHOONIDHI_AUTH_UNAVAILABLE", context: "Bhoonidhi 活跃会话数已达上限" });
  assert.match(message, /会话数已达上限/);
  for (const context of ["password=fixture-only", "https://example.invalid/auth", "C:\\private\\credentials", "generic internal owner"]) {
    assert.doesNotMatch(formatError({ code: "BHOONIDHI_AUTH_UNAVAILABLE", context }), /fixture-only|example\.invalid|private|internal owner/);
  }
});

test("native DEM and runtime categories do not disappear behind the generic retry text", () => {
  const source = readFileSync(new URL("../../../crates/insar-application-service/src/lib.rs", import.meta.url), "utf8");
  const cases = [
    ["PROCESSING_ENGINE_UNAVAILABLE", /处理引擎/],
    ["V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE", /后台.*未就绪/],
    ["V3_DOWNLOAD_INITIALIZATION_FAILED", /下载.*初始化失败/],
    ["V3_STORE_NOT_READY", /任务数据库.*未就绪/],
    ["V3_STORE_PATH_REQUIRED", /任务数据库.*目录/],
    ["V3_APPLICATION_STATE_POISONED", /后台.*状态异常/],
    ["V3_PROVIDER_OPERATION_FAILED", /数据源.*具体原因/],
  ] as const;
  for (const [code, expected] of cases) {
    assert.ok(source.includes(`\"${code}\"`));
    const message = formatError({ code });
    assert.match(message, expected, code);
    assert.doesNotMatch(message, /操作失败，请稍后重试|Sentinel-2|Landsat|USGS/);
  }
});

test("exact native DEM validation reasons get fixed translations without exposing the raw context", () => {
  const source = readFileSync(new URL("../../../crates/insar-application-service/src/lib.rs", import.meta.url), "utf8");
  const cases = [
    ["a current persisted AOI is required for a DEM task", /保存.*AOI/],
    ["DEM AOI geometry or display identity is stale relative to Store authority", /AOI.*变化|范围.*变化/],
    ["persisted DEM AOI geometry is invalid", /AOI.*无效/],
    ["persisted DEM AOI cannot derive request windows", /AOI.*请求范围/],
  ] as const;
  for (const [context, expected] of cases) {
    assert.ok(source.includes(context));
    const message = formatError({ code: "V3_APPLICATION_INVALID_REQUEST", context });
    assert.match(message, expected, context);
    assert.doesNotMatch(message, /Store|persisted|authority/);
  }
  assert.doesNotMatch(formatError({ code: "V3_APPLICATION_INVALID_REQUEST", context: "persisted DEM AOI geometry is invalid password=fixture-only" }), /fixture-only/);
});
