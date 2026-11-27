import assert from "node:assert/strict";
import test from "node:test";

import {
  nisarSubsetFailuresAreLocal,
  nisarSubsetFailuresAreNetwork,
  nisarSubsetFailuresAreTlsCertificate,
  nisarSubsetRetryHint,
} from "./nisarSubsetRetryHint.ts";

const LOGIN = "检测登录";
const NETWORK_HINT = "失败任务与断点已保留。网络请求超时或连接中断，请检查网络后重试。";

test("a local budget-upgrade refusal keeps the retained-data note without the login step", () => {
  const status = { failure_codes: [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }] };
  assert.equal(nisarSubsetFailuresAreLocal(status), true);
  assert.equal(nisarSubsetRetryHint(status), "失败任务与断点已保留。");
});

test("Store refusals and local-only diagnostics are local causes", () => {
  for (const failure_codes of [
    [{ code: "V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED", local: false }],
    [{ code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: true }],
    [{ code: "NISAR_SUBSET_V2_RECOVERY_INVALID", local: false }, { code: "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH", local: false }],
  ]) {
    assert.ok(!nisarSubsetRetryHint({ failure_codes }).includes(LOGIN), JSON.stringify(failure_codes));
  }
});

// Issue #8 slice 6: every writer-stage and output-admission failure is a
// local result (no provider answered it); an older host may not flag it local,
// so the stable code alone must suppress the login step.
const LOCAL_WRITER_CODES = [
  "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT",
  "NISAR_SUBSET_V2_WRITER_DISK_SPACE_INSUFFICIENT",
  "NISAR_SUBSET_V2_SELECTED_SCIENCE_MISSING",
  "NISAR_SUBSET_V2_MULTI_POL_WRITER_INVALID",
  "NISAR_SUBSET_V2_POST_WRITE_SCIENCE_VALIDATION",
  "NISAR_SUBSET_V2_MANIFEST_INVALID",
  "NISAR_SUBSET_V2_ATOMIC_PROMOTION_FAILED",
  "NISAR_SUBSET_V2_ARTIFACT_INVALID",
  "NISAR_SUBSET_V2_SIDECAR_INVALID",
  "NISAR_SUBSET_V2_FROZEN_PLAN_INVALID",
  "NISAR_SUBSET_ARTIFACT_INVALID",
  "NISAR_SUBSET_V2_OUTPUT_BUSY",
];

test("writer and output-admission failures never ask for a login check", () => {
  for (const code of LOCAL_WRITER_CODES) {
    const status = { failure_codes: [{ code, local: false }] };
    assert.equal(nisarSubsetFailuresAreLocal(status), true, code);
    assert.equal(nisarSubsetRetryHint(status), "失败任务与断点已保留。", code);
  }
  // A writer failure next to an authentication failure still keeps the step.
  assert.equal(
    nisarSubsetFailuresAreLocal({
      failure_codes: [
        { code: "NISAR_SUBSET_V2_WRITER_CAPTURE_LIMIT", local: false },
        { code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: false },
      ],
    }),
    false,
  );
});

test("an authentication failure, an unknown code or absent facts keep the login step", () => {
  for (const status of [
    { failure_codes: [{ code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: false }] },
    // A transport failure next to a possible authentication failure keeps the step.
    { failure_codes: [{ code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false }, { code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: false }] },
    { failure_codes: [{ code: "MORE_FAILURE_CODES", local: false }] },
    { failure_codes: [] },
    { failure_codes: null },
    {},
  ]) {
    assert.equal(nisarSubsetFailuresAreLocal(status), false, JSON.stringify(status));
    assert.equal(nisarSubsetRetryHint(status), "失败任务与断点已保留。认证失败时，请先在设置中检测登录，再重试。");
  }
});

// Downloader enhancement D1c (L4 2026-10-18): a preflight request timeout
// (NISAR_SUBSET_V2_TRANSPORT_FAILED, transport_cause=request_timeout) showed the
// login step. A transport failure has no usable HTTP response and is never an
// authentication failure, so the card asks for a network check; next to a local
// refusal it still does (this case asked for a login check before D1c).
test("a transport failure asks for a network check instead of a login check", () => {
  for (const failure_codes of [
    [{ code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false }],
    [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }, { code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false }],
  ]) {
    const status = { failure_codes };
    assert.equal(nisarSubsetFailuresAreLocal(status), false, JSON.stringify(failure_codes));
    assert.equal(nisarSubsetFailuresAreNetwork(status), true, JSON.stringify(failure_codes));
    assert.equal(nisarSubsetRetryHint(status), NETWORK_HINT, JSON.stringify(failure_codes));
    assert.ok(!nisarSubsetRetryHint(status).includes(LOGIN));
  }
  // Only local failures need no step at all.
  assert.equal(nisarSubsetFailuresAreNetwork({ failure_codes: [{ code: "NISAR_SUBSET_V2_RECOVERY_INVALID", local: false }] }), false);
});

// L5 recheck 2026-10-19: every NISAR Range request failed because the server
// certificate could not be verified (a low-integrity process could not open the
// user's certificate store). Such a failure is neither a login nor a timeout problem.
test("a certificate verification failure names the certificate problem", () => {
  const CERTIFICATE_HINT = "失败任务与断点已保留。无法验证服务器证书（本机证书库无法读取或网络被拦截），请检查网络环境或系统证书设置后重试。";
  for (const failure_codes of [
    [{ code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false, tls_certificate: true }],
    [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }, { code: "DL005", local: false, tls_certificate: true }],
  ]) {
    const status = { failure_codes };
    assert.equal(nisarSubsetFailuresAreTlsCertificate(status), true, JSON.stringify(failure_codes));
    assert.equal(nisarSubsetRetryHint(status), CERTIFICATE_HINT, JSON.stringify(failure_codes));
    assert.ok(!nisarSubsetRetryHint(status).includes(LOGIN));
  }
  // Next to a transport failure without that cause, the network check stays;
  // next to a possible authentication failure, the login step stays.
  assert.equal(
    nisarSubsetRetryHint({ failure_codes: [
      { code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false, tls_certificate: true },
      { code: "NISAR_SUBSET_V2_TRANSPORT_FAILED", local: false },
    ] }),
    NETWORK_HINT,
  );
  assert.equal(
    nisarSubsetRetryHint({ failure_codes: [
      { code: "DL005", local: false, tls_certificate: true },
      { code: "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED", local: false },
    ] }),
    "失败任务与断点已保留。认证失败时，请先在设置中检测登录，再重试。",
  );
});

// Downloader enhancement D1d (L4 2026-10-18): a Bhoonidhi download-probe
// timeout failed the item as DL005 and the card asked for a login check. DL005
// is no authentication code; when the failure's diagnostic declares a network
// transport cause (`source=network`, projected by the Store as `network`), the
// card asks for a network check. An authentication failure keeps the login step.
test("a DL005 failure with a network transport cause asks for a network check", () => {
  for (const failure_codes of [
    [{ code: "DL005", local: false, network: true }],
    [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }, { code: "DL005", local: false, network: true }],
  ]) {
    const status = { failure_codes };
    assert.equal(nisarSubsetFailuresAreNetwork(status), true, JSON.stringify(failure_codes));
    assert.equal(nisarSubsetRetryHint(status), NETWORK_HINT, JSON.stringify(failure_codes));
  }
  for (const failure_codes of [
    [{ code: "DL004", local: false }],
    [{ code: "DL005", local: false }],
    [{ code: "DL005", local: false, network: false }],
    [{ code: "DL005", local: false, network: true }, { code: "DL004", local: false }],
  ]) {
    assert.equal(
      nisarSubsetRetryHint({ failure_codes }),
      "失败任务与断点已保留。认证失败时，请先在设置中检测登录，再重试。",
      JSON.stringify(failure_codes),
    );
  }
});
