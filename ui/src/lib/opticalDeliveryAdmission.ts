import { t } from "../i18n/index.ts";
import type { DesktopOpticalDeliveryAdmission, DesktopSentinel2ArchiveAdmissionRejection } from "./desktopShellBridge.ts";

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

const REASON_DETAIL_VALUES: Record<string, RegExp> = {
  reason: /^[a-z_]{1,32}$/,
  http_status: /^[1-5][0-9]{2}$/,
  retry_after_seconds: /^[0-9]{1,4}$/,
  retry_after: /^(?:http_date|invalid)$/,
  transport: /^(?:timeout|connect|other)$/,
  attempts: /^[1-9]$/,
};

/** Same allowlist as the application service: bounded `key=value` pairs, never a URL or a secret. */
function validReasonDetail(detail: unknown): detail is string {
  if (typeof detail !== "string" || detail.length < 1 || detail.length > 128) return false;
  const seen = new Set<string>();
  for (const pair of detail.split(";")) {
    const separator = pair.indexOf("=");
    const key = pair.slice(0, separator), value = pair.slice(separator + 1);
    if (separator < 1 || !Object.hasOwn(REASON_DETAIL_VALUES, key) || seen.has(key) || !REASON_DETAIL_VALUES[key].test(value)
      || (key === "retry_after_seconds" && Number(value) > 3600)) return false;
    seen.add(key);
  }
  return true;
}

function reasonDetailValue(detail: string | undefined, key: string): string | undefined {
  return detail?.split(";").map(pair => pair.split("=")).find(([name]) => name === key)?.[1];
}

/** Same bounded wire contract for live admission and a persisted task report. */
export function parseOpticalDeliveryAdmission(value: unknown): DesktopOpticalDeliveryAdmission | null {
  const raw = record(value);
  if (!raw || Object.keys(raw).some(key => !["requested_count", "admitted_count", "rejected_count", "rejections"].includes(key))) return null;
  const { requested_count: requested, admitted_count: admitted, rejected_count: rejected, rejections } = raw;
  if (typeof requested !== "number" || !Number.isSafeInteger(requested) || requested < 1 || requested > 250
    || typeof admitted !== "number" || !Number.isSafeInteger(admitted) || admitted < 0
    || typeof rejected !== "number" || !Number.isSafeInteger(rejected) || rejected < 0
    || admitted + rejected !== requested || !Array.isArray(rejections) || rejections.length !== rejected) return null;
  const ids = new Set<string>();
  const parsed: DesktopOpticalDeliveryAdmission["rejections"] = [];
  for (const entry of rejections) {
    const row = record(entry);
    if (!row || Object.keys(row).some(key => !["source_item_id", "reason_code", "reason_detail"].includes(key))
      || typeof row.source_item_id !== "string" || row.source_item_id.length < 1 || row.source_item_id.length > 2048
      || /[\u0000-\u001f\u007f]/.test(row.source_item_id) || row.source_item_id.trim() !== row.source_item_id
      || ids.has(row.source_item_id) || typeof row.reason_code !== "string" || !/^[A-Z0-9_]{1,128}$/.test(row.reason_code)
      || ("reason_detail" in row && !validReasonDetail(row.reason_detail))) return null;
    ids.add(row.source_item_id);
    parsed.push({ source_item_id: row.source_item_id, reason_code: row.reason_code,
      ...(validReasonDetail(row.reason_detail) ? { reason_detail: row.reason_detail } : {}) });
  }
  return { requested_count: requested, admitted_count: admitted, rejected_count: rejected, rejections: parsed };
}

export function opticalAdmissionReason(code: string, detail?: string): string {
  if (code === "MPC_ASSET_SIGN_RATE_LIMITED") {
    const seconds = Number(reasonDetailValue(detail, "retry_after_seconds"));
    if (seconds >= 120) {
      return t("shared.opticalAdmission.mpcRateLimitMinutes", { minutes: Math.ceil(seconds / 60) });
    }
    return seconds > 0
      ? t("shared.opticalAdmission.mpcRateLimitSeconds", { seconds })
      : t("shared.opticalAdmission.mpcRateLimitLater");
  }
  if (code === "MPC_ASSET_SIGN_UNAVAILABLE") return t("shared.opticalAdmission.mpcUnavailable");
  if (code === "MPC_ASSET_SIGN_TRANSPORT") return t("shared.opticalAdmission.mpcTransport");
  if (code === "MPC_ASSET_SIGN_REJECTED") return t("shared.opticalAdmission.mpcRejected");
  if (code === "MPC_ASSET_SIGN_RESPONSE_INVALID") return t("shared.opticalAdmission.mpcResponseInvalid");
  if (code === "MPC_ASSET_SIGN_FAILED") return t("shared.opticalAdmission.mpcFailed");
  if (code === "MPC_ASSET_ITEM_FAILED") return t("shared.opticalAdmission.mpcItemFailed");
  if (code === "MPC_ASSET_PROBE_FAILED") return t("shared.opticalAdmission.mpcProbeFailed");
  if (code === "USGS_M2M_DOWNLOAD_UNAVAILABLE") return t("shared.opticalAdmission.usgsUnavailable");
  if (code.includes("CANCEL")) return t("shared.opticalAdmission.cancelled");
  if (code.includes("IDENTITY") || code.includes("EVIDENCE") || code.includes("MANIFEST")) {
    return t("shared.opticalAdmission.identityOrManifestFailed");
  }
  if (code.includes("TIMEOUT")) return t("shared.opticalAdmission.timeout");
  if (code.includes("AUTH") || code.includes("LOGIN")) return t("shared.opticalAdmission.authRequired");
  if (code.includes("UNAVAILABLE") || code.includes("NOT_FOUND")) return t("shared.opticalAdmission.productUnavailable");
  if (code.includes("NETWORK") || code.includes("HTTP") || code.includes("LOOKUP")) return t("shared.opticalAdmission.lookupFailed");
  return t("shared.opticalAdmission.preparationFailed");
}

/** The code plus its bounded detail, kept visible so a report can be quoted as evidence. */
export function opticalAdmissionDiagnostic(row: DesktopSentinel2ArchiveAdmissionRejection): string {
  return row.reason_detail ? `${row.reason_code}；${row.reason_detail}` : row.reason_code;
}

export function opticalAdmissionFailureMessage(admission: DesktopOpticalDeliveryAdmission): string {
  return t("shared.opticalAdmission.failureMessageHeader", { count: admission.rejected_count }) + admission.rejections.map(row =>
    t("shared.opticalAdmission.failureMessageRow", {
      itemId: row.source_item_id.split("::ITEM::").at(-1) ?? "",
      reason: opticalAdmissionReason(row.reason_code, row.reason_detail),
      diagnostic: opticalAdmissionDiagnostic(row),
    }),
  ).join("\n");
}
