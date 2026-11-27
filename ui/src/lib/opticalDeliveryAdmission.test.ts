import assert from "node:assert/strict";
import test from "node:test";
import { opticalAdmissionFailureMessage, opticalAdmissionReason, parseOpticalDeliveryAdmission } from "./opticalDeliveryAdmission.ts";

const partial = { requested_count: 2, admitted_count: 1, rejected_count: 1,
  rejections: [{ source_item_id: "scene-b", reason_code: "LANDSAT_PUBLIC_ASSET_FROZEN_EVIDENCE_INVALID" }] };
test("admission keeps scene and file counts separate, including an all-rejected response", () => {
  assert.deepEqual(parseOpticalDeliveryAdmission(partial), partial);
  const all = { ...partial, admitted_count: 0, requested_count: 1 };
  assert.deepEqual(parseOpticalDeliveryAdmission(all), all);
  assert.match(opticalAdmissionFailureMessage(all), /scene-b/);
});
test("provider preparation errors use actionable delivery wording without exposing internals", () => {
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_FAILED"), "MPC 临时服务响应失败，可重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_ITEM_FAILED"), "MPC 产品信息读取失败");
  assert.equal(opticalAdmissionReason("MPC_ASSET_PROBE_FAILED"), "MPC 远端文件校验失败");
  assert.equal(opticalAdmissionReason("USGS_M2M_DOWNLOAD_UNAVAILABLE"), "USGS 当前未提供官方产品包");
});

test("admission rejects forged counts, extra secret fields, duplicate identities and raw reason text", () => {
  for (const value of [null, { ...partial, requested_count: 0 }, { ...partial, admitted_count: 2 },
    { ...partial, rejected_count: 2 }, { ...partial, extra: "secret" },
    { ...partial, rejections: [{ ...partial.rejections[0], url: "not-allowed" }] },
    { ...partial, rejections: [{ ...partial.rejections[0], reason_code: "https://example.invalid" }] },
    { requested_count: 2, admitted_count: 0, rejected_count: 2, rejections: [partial.rejections[0], partial.rejections[0]] },
  ]) assert.equal(parseOpticalDeliveryAdmission(value), null);
});

const rateLimitedDetail = "reason=http_status_transient;http_status=429;retry_after_seconds=3;attempts=4";
const rateLimited = { requested_count: 2, admitted_count: 0, rejected_count: 2, rejections: [
  { source_item_id: "PROVIDER::mpc::ITEM::scene-a", reason_code: "MPC_ASSET_SIGN_RATE_LIMITED", reason_detail: rateLimitedDetail },
  { source_item_id: "PROVIDER::mpc::ITEM::scene-b", reason_code: "MPC_ASSET_SIGN_REJECTED", reason_detail: "reason=http_status_terminal;http_status=403;attempts=1" },
] };

test("MPC sign failures keep their class and bounded detail in the admission report", () => {
  assert.deepEqual(parseOpticalDeliveryAdmission(rateLimited), rateLimited);
  const message = opticalAdmissionFailureMessage(rateLimited);
  assert.match(message, /scene-a：MPC 签名服务限流，请约 3 秒后重试（MPC_ASSET_SIGN_RATE_LIMITED；.*retry_after_seconds=3/);
  assert.match(message, /scene-b：MPC 拒绝为该文件签名，重试无效（MPC_ASSET_SIGN_REJECTED；.*http_status=403/);
});

test("MPC sign failure wording says whether retrying can help", () => {
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_RATE_LIMITED", rateLimitedDetail), "MPC 签名服务限流，请约 3 秒后重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_RATE_LIMITED", "reason=http_status_transient;http_status=429;retry_after_seconds=3600;attempts=4"),
    "MPC 签名服务限流，请约 60 分钟后重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_RATE_LIMITED"), "MPC 签名服务限流，请稍后重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_UNAVAILABLE"), "MPC 签名服务暂时不可用，可稍后重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_TRANSPORT"), "与 MPC 签名服务的连接失败，请检查网络后重试");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_REJECTED"), "MPC 拒绝为该文件签名，重试无效");
  assert.equal(opticalAdmissionReason("MPC_ASSET_SIGN_RESPONSE_INVALID"), "MPC 签名响应未通过校验");
});

test("admission refuses a reason detail outside the allowlist", () => {
  const overlongReason = `reason=${"a".repeat(33)}`;
  for (const detail of ["", "https://example.invalid/sign?href=x", "reason=transport;sig=secret", "token=abc",
    "reason=Transport", "reason=transport;reason=transport", "http_status=600", "retry_after_seconds=3601",
    "retry_after=tomorrow", "transport=dns", "attempts=0", "attempts=10", "reason=response_json;", overlongReason, 7]) {
    const value = { ...partial, rejections: [{ ...partial.rejections[0], reason_detail: detail }] };
    assert.equal(parseOpticalDeliveryAdmission(value), null, String(detail));
  }
});
