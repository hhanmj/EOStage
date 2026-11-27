import { formatBridgeErrorValue } from "../../lib/bridgeErrorFormat.ts";
import { explainErrorCode, lookupErrorText } from "./errorExplanations.ts";
import type { AccountId, TypedError } from "./types.ts";

export interface OpticalCreateErrorResolution extends TypedError {
  code: string;
  message_zh: string;
  needs_account: AccountId | null;
  actionAccountId: AccountId | null;
  actionLabel: string | null;
  retryable: false;
  needs_user_confirmation: false;
}

export function inferRequiredAccountFromCode(code?: string | null): AccountId | null {
  if (!code || typeof code !== "string") return null;
  const normalized = code.trim().toUpperCase();
  if (!normalized) return null;
  if (
    normalized === "CDSE_ACCOUNT_LOGIN_REQUIRED" ||
    normalized === "CDSE_CREDENTIALS_MISSING" ||
    normalized.startsWith("CDSE_OAUTH_") ||
    normalized.startsWith("CDSE_AUTH_") ||
    normalized === "CDSE_ODATA_HTTP_UNAUTHORIZED" ||
    normalized === "CDSE_ODATA_HTTP_FORBIDDEN"
  ) {
    return "cdse";
  }
  if (
    normalized === "EARTHDATA_ACCOUNT_LOGIN_REQUIRED" ||
    normalized === "EARTHDATA_CREDENTIALS_MISSING" ||
    normalized === "EARTHDATA_TOKEN_INVALID" ||
    normalized === "EARTHDATA_UNAUTHORIZED" ||
    normalized === "ASF_UNAUTHORIZED" ||
    normalized.startsWith("EARTHDATA_AUTH_")
  ) {
    return "earthdata";
  }
  if (
    normalized === "USGS_ACCOUNT_LOGIN_REQUIRED" ||
    normalized === "USGS_CREDENTIALS_MISSING" ||
    normalized === "USGS_M2M_UNAUTHORIZED" ||
    normalized.startsWith("USGS_M2M_AUTH_")
  ) {
    return "usgs_m2m";
  }
  if (
    normalized === "BHOONIDHI_ACCOUNT_LOGIN_REQUIRED" ||
    normalized === "BHOONIDHI_CREDENTIALS_MISSING" ||
    normalized.startsWith("BHOONIDHI_AUTH_") ||
    normalized === "BHOONIDHI_UNAUTHORIZED" ||
    normalized === "BHOONIDHI_FORBIDDEN"
  ) {
    return "bhoonidhi";
  }
  if (
    normalized === "OPENTOPOGRAPHY_ACCOUNT_LOGIN_REQUIRED" ||
    normalized === "OPENTOPOGRAPHY_CREDENTIALS_MISSING" ||
    normalized === "OPENTOPOGRAPHY_UNAUTHORIZED"
  ) {
    return "opentopography";
  }
  return null;
}

export function defaultAccountActionLabel(accountId: AccountId): string {
  switch (accountId) {
    case "cdse":
      return "前往配置 CDSE 账号";
    case "usgs_m2m":
      return "前往配置 USGS M2M 账号";
    case "earthdata":
      return "前往配置 Earthdata 账号";
    case "bhoonidhi":
      return "前往配置 Bhoonidhi 账号";
    case "opentopography":
      return "前往配置 OpenTopography 账号";
    case "gacos":
      return "前往配置 GACOS 账号";
    default:
      return "前往配置数据账号";
  }
}

function inferAccountFromMessageAndResource(
  message?: string | null,
  fallbackResourceId?: string | null,
): AccountId | null {
  const text = typeof message === "string" ? message.trim() : "";
  if (!text) return null;
  const lower = text.toLowerCase();
  const looksAuthOrSession =
    /无法建立下载会话|凭据|未登录|需要登录|重新登录|登录已过期|登录失效|授权失败|认证失败|身份验证|api\s*key|unauthorized|forbidden|401|403|token/i.test(
      text,
    );
  if (!looksAuthOrSession) return null;

  if (lower.includes("usgs") || lower.includes("m2m")) return "usgs_m2m";
  if (lower.includes("earthdata") || lower.includes("asf") || lower.includes("urs.earthdata")) return "earthdata";
  if (lower.includes("cdse") || lower.includes("copernicus data space")) return "cdse";
  if (lower.includes("bhoonidhi") || lower.includes("nrsc")) return "bhoonidhi";
  if (lower.includes("opentopography")) return "opentopography";
  if (lower.includes("gacos")) return "gacos";

  if (fallbackResourceId) {
    const res = fallbackResourceId.toLowerCase();
    if (res.includes("landsat")) return "usgs_m2m";
    if (res.includes("sentinel-2") || res.includes("sentinel-3")) return "cdse";
    if (res.includes("bhoonidhi") || res.includes("nisar.s")) return "bhoonidhi";
    if (
      res.includes("sentinel-1") ||
      res.includes("nisar") ||
      res.includes("opera") ||
      res.includes("aria") ||
      res.includes("alos")
    ) {
      return "earthdata";
    }
    if (res.includes("opentopography") || res.includes("nasadem") || res.includes("aw3d") || res.includes("srtm")) {
      return "opentopography";
    }
  }
  return null;
}

function isBhoonidhiStaleProductRejection(
  code: string | null | undefined,
  message: string,
): boolean {
  if (inferRequiredAccountFromCode(code) !== null) return false;
  if (typeof code === "string" && code.trim().toUpperCase() === "DL004") return false;
  return (
    /bhoonidhi/i.test(message) &&
    /(重新检索|更新编号|重新处理|re-?search|reprocessed|updated product id)/i.test(message)
  );
}

function isExplicitNonAuthErrorCode(code?: string | null): boolean {
  if (!code || typeof code !== "string") return false;
  const normalized = code.trim().toUpperCase();
  if (!normalized || inferRequiredAccountFromCode(normalized) !== null) return false;
  return (
    normalized.endsWith("_TRANSPORT_FAILED") ||
    normalized.endsWith("_TIMEOUT") ||
    normalized.endsWith("_RATE_LIMITED") ||
    normalized.endsWith("_UNAVAILABLE")
  );
}

export function resolveAccountActionFromError(
  cause: unknown,
  fallbackResourceId?: string | null,
): { actionAccountId: AccountId | null; actionLabel: string | null } {
  const obj = cause && typeof cause === "object" ? (cause as Record<string, unknown>) : null;
  const explicitAccount =
    obj && typeof obj.actionAccountId === "string" && obj.actionAccountId.trim()
      ? (obj.actionAccountId.trim() as AccountId)
      : obj && typeof obj.needs_account === "string" && obj.needs_account.trim()
        ? (obj.needs_account.trim() as AccountId)
        : null;
  const explicitLabel =
    obj && typeof obj.actionLabel === "string" && obj.actionLabel.trim()
      ? obj.actionLabel.trim()
      : null;
  if (explicitAccount) {
    return {
      actionAccountId: explicitAccount,
      actionLabel: explicitLabel || defaultAccountActionLabel(explicitAccount),
    };
  }

  const rawCode =
    obj && typeof obj.code === "string"
      ? obj.code
      : obj && typeof obj.error_code === "string"
        ? obj.error_code
        : null;
  const fromCode = inferRequiredAccountFromCode(rawCode);
  if (fromCode) {
    return {
      actionAccountId: fromCode,
      actionLabel: explicitLabel || defaultAccountActionLabel(fromCode),
    };
  }
  if (isExplicitNonAuthErrorCode(rawCode)) {
    return { actionAccountId: null, actionLabel: null };
  }

  const msg =
    cause instanceof Error
      ? cause.message
      : typeof cause === "string"
        ? cause
        : obj
          ? [obj.message_zh, obj.error, obj.message, obj.primary, obj.secondary]
              .filter((v): v is string => typeof v === "string" && v.trim().length > 0)
              .join(" ")
          : "";
  if (isBhoonidhiStaleProductRejection(rawCode, msg)) {
    return { actionAccountId: null, actionLabel: null };
  }
  const fromMsg = inferAccountFromMessageAndResource(msg, fallbackResourceId);
  if (fromMsg) {
    return {
      actionAccountId: fromMsg,
      actionLabel: explicitLabel || defaultAccountActionLabel(fromMsg),
    };
  }
  return { actionAccountId: null, actionLabel: null };
}

const OPTICAL_BAND_CODE_MESSAGES: Readonly<Record<string, string>> = {
  STAC_ITEM_NOT_FOUND:
    "免账号分析波段镜像（MPC）暂未收录该景影像（最新采集影像通常存在同步延迟）。",
  SENTINEL2_BAND_FROZEN_EVIDENCE_INVALID:
    "该景 Sentinel-2 的免账号分析波段（MPC）校验未通过，可能尚未同步或检索快照已变化。",
  SENTINEL2_BAND_REQUEST_DUPLICATE:
    "同一景 Sentinel-2 的分析波段下载请求正在处理中，请稍候。",
  SENTINEL2_BAND_CANCELLED:
    "已取消该景 Sentinel-2 的分析波段下载准备。",
  OPTICAL_ADMISSION_ALL_REJECTED:
    "所选光学影像均未通过免账号分析波段准备（可能暂未收录或远端签名失败）。",
};

const SENTINEL2_RESOURCES = new Set([
  "science.sentinel-2.l2a.v1",
  "science.sentinel-2.l1c.v1",
]);

function isCancelledOrDuplicateCode(code: string): boolean {
  return (
    code === "SENTINEL2_BAND_CANCELLED" ||
    code === "SENTINEL2_BAND_REQUEST_DUPLICATE" ||
    code === "SENTINEL2_ARCHIVE_CANCELLED" ||
    code === "SENTINEL2_ARCHIVE_REQUEST_DUPLICATE" ||
    code === "STAC_SEARCH_CANCELLED" ||
    code === "PLANETARY_COMPUTER_CANCELLED"
  );
}

/**
 * 将光学（Sentinel-2 / HLS / Landsat）创建下载任务返回的后端错误解析为：
 * 1. 若属于明确的平台认证错误码，标记 `needs_account` 直接唤起对应平台授权弹窗；
 * 2. 否则返回具体可读的中文失败原因，并在 Sentinel-2 未配置 CDSE 时附加明确引导文案与配置按钮元数据。
 */
export function resolveOpticalTaskCreationError(input: {
  code?: string | null;
  error?: string | null;
  context?: string | null;
  resourceId?: string | null;
  deliveryForm?: "safe_archive" | "analysis_bands" | null;
  cdseConfigured?: boolean;
}): OpticalCreateErrorResolution {
  const code =
    typeof input.code === "string" && input.code.trim().length > 0
      ? input.code.trim()
      : "V3_APPLICATION_UNAVAILABLE";
  const contextText =
    typeof input.context === "string" && input.context.trim().length > 0
      ? input.context.trim()
      : null;

  const needsAccount = inferRequiredAccountFromCode(code);
  if (needsAccount) {
    const messageZh =
      explainErrorCode(code) ||
      lookupErrorText("bridge", code) ||
      formatBridgeErrorValue({
        code,
        error: input.error,
        ...(contextText ? { context: contextText } : {}),
      });
    return {
      code,
      message_zh: messageZh,
      needs_account: needsAccount,
      actionAccountId: needsAccount,
      actionLabel: defaultAccountActionLabel(needsAccount),
      retryable: false,
      needs_user_confirmation: false,
    };
  }

  const isSentinel2 =
    typeof input.resourceId === "string" && SENTINEL2_RESOURCES.has(input.resourceId);

  let messageZh: string | null = null;

  if (code === "V3_PROVIDER_OPERATION_FAILED") {
    if (contextText) {
      messageZh = formatBridgeErrorValue({
        code,
        error: input.error,
        context: contextText,
      });
    } else if (isSentinel2 && input.deliveryForm === "analysis_bands") {
      messageZh =
        "该景 Sentinel-2 的免账号分析波段（MPC）暂不可用或尚未收录（最新采集影像通常存在同步延迟）。";
    } else {
      messageZh =
        lookupErrorText("bridge", "V3_PROVIDER_OPERATION_FAILED") ||
        "数据源操作未完成，请核对诊断记录或稍后重试。";
    }
  } else {
    messageZh =
      explainErrorCode(code) ||
      lookupErrorText("bridge", code) ||
      OPTICAL_BAND_CODE_MESSAGES[code] ||
      null;
  }

  if (!messageZh) {
    messageZh = formatBridgeErrorValue({
      code,
      error: input.error,
      ...(contextText ? { context: contextText } : {}),
    });
  }

  let actionAccountId: AccountId | null = null;
  let actionLabel: string | null = null;

  if (
    isSentinel2 &&
    input.cdseConfigured === false &&
    !isCancelledOrDuplicateCode(code)
  ) {
    const trimmed = messageZh.replace(/[。；\s]+$/, "");
    messageZh = `${trimmed}。如需直接下载官方完整 SAFE 归档，请先配置 Copernicus Data Space (CDSE) 账号。`;
    actionAccountId = "cdse";
    actionLabel = "前往配置 CDSE 账号";
  }

  return {
    code,
    message_zh: messageZh,
    needs_account: null,
    actionAccountId,
    actionLabel,
    retryable: false,
    needs_user_confirmation: false,
  };
}
