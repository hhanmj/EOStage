import assert from "node:assert/strict";
import test from "node:test";
import { describeDownloadFailure } from "./errorExplanations.ts";
import {
  defaultAccountActionLabel,
  inferRequiredAccountFromCode,
  resolveAccountActionFromError,
  resolveOpticalTaskCreationError,
} from "./opticalCreateError.ts";

test("inferRequiredAccountFromCode & resolveAccountActionFromError cover USGS M2M, Earthdata, CDSE, Bhoonidhi, and OpenTopography", () => {
  assert.equal(inferRequiredAccountFromCode("EARTHDATA_CREDENTIALS_MISSING"), "earthdata");
  assert.equal(inferRequiredAccountFromCode("EARTHDATA_UNAUTHORIZED"), "earthdata");
  assert.equal(inferRequiredAccountFromCode("ASF_UNAUTHORIZED"), "earthdata");
  assert.equal(inferRequiredAccountFromCode("USGS_M2M_UNAUTHORIZED"), "usgs_m2m");
  assert.equal(inferRequiredAccountFromCode("OPENTOPOGRAPHY_UNAUTHORIZED"), "opentopography");

  const usgsErr = new Error("USGS M2M 已保存凭据当前无法建立下载会话，请检查账号状态后重试。");
  assert.deepEqual(resolveAccountActionFromError(usgsErr, "science.landsat.c2-l2.v1"), {
    actionAccountId: "usgs_m2m",
    actionLabel: defaultAccountActionLabel("usgs_m2m"),
  });

  const earthdataErr = { code: "EARTHDATA_TOKEN_INVALID", message_zh: "Earthdata 登录已过期" };
  assert.deepEqual(resolveAccountActionFromError(earthdataErr, "science.sentinel-1.standard.v1"), {
    actionAccountId: "earthdata",
    actionLabel: defaultAccountActionLabel("earthdata"),
  });

  const nonAuthErr = new Error("上游服务暂时超时 504");
  assert.deepEqual(resolveAccountActionFromError(nonAuthErr, "science.landsat.c2-l2.v1"), {
    actionAccountId: null,
    actionLabel: null,
  });

  const transportErr = {
    code: "USGS_M2M_TRANSPORT_FAILED",
    message_zh:
      "无法建立到 USGS M2M 的网络连接。请检查 DNS、系统代理和 TLS/证书链；这不是已确认的 Application Token 错误。",
  };
  assert.deepEqual(resolveAccountActionFromError(transportErr, "science.landsat.c2-l2.v1"), {
    actionAccountId: null,
    actionLabel: null,
  });
});

test("inferRequiredAccountFromCode maps auth error codes to canonical account IDs", () => {
  assert.equal(inferRequiredAccountFromCode("CDSE_ACCOUNT_LOGIN_REQUIRED"), "cdse");
  assert.equal(inferRequiredAccountFromCode("CDSE_OAUTH_REJECTED"), "cdse");
  assert.equal(inferRequiredAccountFromCode("CDSE_ODATA_HTTP_UNAUTHORIZED"), "cdse");
  assert.equal(inferRequiredAccountFromCode("CDSE_ODATA_HTTP_FORBIDDEN"), "cdse");
  assert.equal(inferRequiredAccountFromCode("EARTHDATA_ACCOUNT_LOGIN_REQUIRED"), "earthdata");
  assert.equal(inferRequiredAccountFromCode("USGS_ACCOUNT_LOGIN_REQUIRED"), "usgs_m2m");
  assert.equal(inferRequiredAccountFromCode("USGS_M2M_AUTH_REQUIRED"), "usgs_m2m");
  assert.equal(inferRequiredAccountFromCode("BHOONIDHI_CREDENTIALS_MISSING"), "bhoonidhi");
  assert.equal(inferRequiredAccountFromCode("OPENTOPOGRAPHY_ACCOUNT_LOGIN_REQUIRED"), "opentopography");
  assert.equal(inferRequiredAccountFromCode("STAC_ITEM_NOT_FOUND"), null);
  assert.equal(inferRequiredAccountFromCode(null), null);
});

test("Sentinel-2 MPC STAC_ITEM_NOT_FOUND & V3_PROVIDER_OPERATION_FAILED produce clear reason + CDSE guidance when CDSE is unconfigured", () => {
  const fromStacNotFound = resolveOpticalTaskCreationError({
    code: "STAC_ITEM_NOT_FOUND",
    error: "该目录产品暂无可用数据",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "analysis_bands",
    cdseConfigured: false,
  });
  assert.equal(fromStacNotFound.needs_account, null);
  assert.ok(fromStacNotFound.message_zh.includes("MPC"));
  assert.ok(fromStacNotFound.message_zh.includes("同步延迟"));
  assert.ok(fromStacNotFound.message_zh.includes("Copernicus Data Space (CDSE)"));
  assert.equal(fromStacNotFound.actionAccountId, "cdse");
  assert.equal(fromStacNotFound.actionLabel, "前往配置 CDSE 账号");

  const fromProviderFallbackNoContext = resolveOpticalTaskCreationError({
    code: "V3_PROVIDER_OPERATION_FAILED",
    error: "当前操作未能取得有效的后台响应，暂时无法确认结果。",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "analysis_bands",
    cdseConfigured: false,
  });
  assert.ok(fromProviderFallbackNoContext.message_zh.includes("免账号分析波段（MPC）"));
  assert.ok(fromProviderFallbackNoContext.message_zh.includes("同步延迟"));
  assert.ok(fromProviderFallbackNoContext.message_zh.includes("Copernicus Data Space (CDSE)"));
  assert.equal(fromProviderFallbackNoContext.actionAccountId, "cdse");
  assert.equal(fromProviderFallbackNoContext.actionLabel, "前往配置 CDSE 账号");

  const fromFrozenInvalid = resolveOpticalTaskCreationError({
    code: "SENTINEL2_BAND_FROZEN_EVIDENCE_INVALID",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "analysis_bands",
    cdseConfigured: false,
  });
  assert.ok(fromFrozenInvalid.message_zh.includes("校验未通过"));
  assert.equal(fromFrozenInvalid.actionAccountId, "cdse");
});

test("Sentinel-2 CDSE auth code triggers needs_account instead of silent failure", () => {
  const res = resolveOpticalTaskCreationError({
    code: "CDSE_ACCOUNT_LOGIN_REQUIRED",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "safe_archive",
    cdseConfigured: false,
  });
  assert.equal(res.needs_account, "cdse");
  assert.ok(res.message_zh.includes("CDSE"));
});

test("Sentinel-2 when CDSE is already configured does not show redundant CDSE setup action", () => {
  const res = resolveOpticalTaskCreationError({
    code: "STAC_ITEM_NOT_FOUND",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "analysis_bands",
    cdseConfigured: true,
  });
  assert.equal(res.actionAccountId, null);
  assert.equal(res.actionLabel, null);
  assert.equal(res.message_zh.includes("请先配置 Copernicus Data Space"), false);
});

test("cancelled optical error does not attach CDSE configuration button", () => {
  const res = resolveOpticalTaskCreationError({
    code: "SENTINEL2_BAND_CANCELLED",
    resourceId: "science.sentinel-2.l2a.v1",
    deliveryForm: "analysis_bands",
    cdseConfigured: false,
  });
  assert.equal(res.actionAccountId, null);
  assert.equal(res.actionLabel, null);
  assert.ok(res.message_zh.includes("已取消"));
});

test("Bhoonidhi stale product 403 maps to non-auth guidance without account button, while 401/DL004 still prompts account setup", () => {
  const stale403Msg =
    "Bhoonidhi 拒绝下载该影像（HTTP 403）：该景可能已被平台重新处理并更新编号，请重新检索最新结果后再试";
  assert.deepEqual(
    resolveAccountActionFromError(
      { code: "DL005", message_zh: stale403Msg },
      "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1",
    ),
    { actionAccountId: null, actionLabel: null },
  );

  const auth401 = resolveAccountActionFromError(
    { code: "DL004", message_zh: "Bhoonidhi probe returned HTTP status 401" },
    "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1",
  );
  assert.deepEqual(auth401, {
    actionAccountId: "bhoonidhi",
    actionLabel: defaultAccountActionLabel("bhoonidhi"),
  });

  const staleFailure = describeDownloadFailure({
    codes: ["DL005"],
    messageZh: stale403Msg,
  });
  assert.equal(staleFailure.cause, stale403Msg);
  assert.equal(staleFailure.guidance, null);
});
