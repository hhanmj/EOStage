// Retry hint of a failed NISAR subset task card (review 2026-10-17, L4). The
// card asks the user to check the login only when a failure may be an
// authentication failure. A local validation, recovery or Store refusal states
// its local cause itself and never needs that step: every failure is local when
// its stable code is a local recovery/validation code, a Store code, or its
// diagnostic declares a local-only cause (`source=local`, for example the
// local hourly refresh limit). Unknown or absent failure facts keep the hint.
// A transport failure (no usable HTTP response: a request timeout, a refused
// or reset connection) is never an authentication failure; when every
// remaining failure is one, the card asks for a network check instead
// (downloader enhancement D1c, L4 2026-10-18: a single preflight request
// timeout showed the login step). Besides the transport code, a failure whose
// diagnostic declares a network transport cause (`source=network`, projected
// as `network`) is one (downloader enhancement D1d, L4 2026-10-18: a Bhoonidhi
// download-probe timeout failed as DL005 and showed the login step).
// A failure whose server certificate could not be verified (its diagnostic
// declares `source=tls_certificate`, projected as `tls_certificate`) is neither
// an authentication failure nor a timeout; when every remaining failure is one,
// the card names the certificate problem (L5 recheck 2026-10-19: a
// low-integrity process could not open the user's certificate store and every
// request failed this way).
export type NisarSubsetRetryHintInput = {
  failure_codes?: ReadonlyArray<{ code: string; local: boolean; tls_certificate?: boolean; network?: boolean }> | null;
};

const LOCAL_FAILURE_CODES: ReadonlySet<string> = new Set([
  "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED",
  "NISAR_SUBSET_V2_RECOVERY_INVALID",
  "NISAR_SUBSET_V2_WRITER_RECOVERY_INVALID",
  "NISAR_SUBSET_V2_ARCHIVED_RECOVERY_INVALID",
  "NISAR_SUBSET_V2_CHECKPOINT_PERSISTENCE",
  "NISAR_SUBSET_V2_SCIENCE_AUTHORITY_INVALID",
  "NISAR_SUBSET_V2_WRITER_BINDING",
  "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED",
  "NISAR_SUBSET_V2_PLANNING_WORKSPACE_MISSING",
  "NISAR_SUBSET_V2_PLANNING_WORKSPACE_IDENTITY_MISMATCH",
  // Writer-stage and output-admission results (Issue #8, slice 6).
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
]);

const NETWORK_FAILURE_CODES: ReadonlySet<string> = new Set([
  "NISAR_SUBSET_V2_TRANSPORT_FAILED",
]);

const RETAINED = "失败任务与断点已保留。";
const CHECK_LOGIN = "认证失败时，请先在设置中检测登录，再重试。";
const CHECK_NETWORK = "网络请求超时或连接中断，请检查网络后重试。";
const CHECK_CERTIFICATE = "无法验证服务器证书（本机证书库无法读取或网络被拦截），请检查网络环境或系统证书设置后重试。";

function failureIsLocal(failure: { code: string; local: boolean }): boolean {
  return failure.local || LOCAL_FAILURE_CODES.has(failure.code) || failure.code.startsWith("V3_STORE_");
}

export function nisarSubsetFailuresAreLocal(status: NisarSubsetRetryHintInput): boolean {
  const failures = status.failure_codes;
  return Boolean(failures && failures.length > 0) && failures!.every(failureIsLocal);
}

function failureIsNetwork(failure: { code: string; network?: boolean }): boolean {
  return NETWORK_FAILURE_CODES.has(failure.code) || failure.network === true;
}

/** Every failure is local or a network transport failure, and at least one is the latter. */
export function nisarSubsetFailuresAreNetwork(status: NisarSubsetRetryHintInput): boolean {
  const failures = status.failure_codes;
  return Boolean(failures && failures.length > 0)
    && failures!.some((failure) => !failureIsLocal(failure) && failureIsNetwork(failure))
    && failures!.every((failure) => failureIsLocal(failure) || failureIsNetwork(failure));
}

/** Every failure is local or could not verify the server certificate, and at least one is the latter. */
export function nisarSubsetFailuresAreTlsCertificate(status: NisarSubsetRetryHintInput): boolean {
  const failures = status.failure_codes;
  return Boolean(failures && failures.length > 0)
    && failures!.some((failure) => !failureIsLocal(failure) && failure.tls_certificate === true)
    && failures!.every((failure) => failureIsLocal(failure) || failure.tls_certificate === true);
}

export function nisarSubsetRetryHint(status: NisarSubsetRetryHintInput): string {
  if (nisarSubsetFailuresAreLocal(status)) return RETAINED;
  if (nisarSubsetFailuresAreTlsCertificate(status)) return RETAINED + CHECK_CERTIFICATE;
  return nisarSubsetFailuresAreNetwork(status) ? RETAINED + CHECK_NETWORK : RETAINED + CHECK_LOGIN;
}
