import { DEFAULT_LOCALE, LOCALES, getCurrentLocale, t, type MessageKey } from "../i18n/index.ts";
import { MESSAGES } from "../i18n/locales.ts";
import { lookupErrorText } from "../shell/api/errorExplanations.ts";

export type BridgeErrorFormatContext = {
  dataset?: string | null;
};

function isBurstDataset(dataset: string | null | undefined): boolean {
  return String(dataset ?? "").trim().toUpperCase() === "SLC-BURST";
}

const INTERNAL_ERROR_TERM_PATTERN =
  /(ApplicationService|Desktop API|Worker|Tauri|Rust|Sidecar|V3_|GUI\d|requires|does not expose|not implemented|unavailable|Task summary|unknown provider|task-view|snapshot|route|Store|internal owner)/i;
const INTERNAL_CONTEXT_TERM_PATTERN =
  /(ApplicationService|Desktop API|Worker|Tauri|Rust|Sidecar|V3_|GUI\d|does not expose|not implemented|Task summary|task-view|snapshot|Store|internal owner)/i;

function containsInternalErrorTerm(value: string): boolean {
  return INTERNAL_ERROR_TERM_PATTERN.test(value);
}

function containsInternalContextTerm(value: string): boolean {
  return INTERNAL_CONTEXT_TERM_PATTERN.test(value);
}

const DEM_MAX_SIDE = 32 * 1024;
const DEM_MAX_PIXELS = 512 * 1024 * 1024;
const CJK_TEXT = /[㐀-鿿]/;

function demRangeLimitMessage(width: number, height: number, pixels: number, coarser: boolean) {
  // Enough decimals that a range just over the limit never prints the limit's own value.
  let digits = 2;
  while (digits < 6 && pixels !== DEM_MAX_PIXELS
    && (pixels / 1e8).toFixed(digits) === (DEM_MAX_PIXELS / 1e8).toFixed(digits)) {
    digits += 1;
  }
  return t("errors.bridgeText.demRangeLimit", {
    width,
    height,
    pixels: (pixels / 1e8).toFixed(digits),
    maxSide: DEM_MAX_SIDE,
    maxPixels: (DEM_MAX_PIXELS / 1e8).toFixed(digits),
    coarser: coarser ? t("errors.bridgeText.demRangeCoarser") : "",
  });
}

// Exact, source-owned DEM request reasons (backend context strings) and their message keys.
const DEM_INVALID_REQUEST_REASONS = new Map<string, MessageKey>([
  ["a current persisted AOI is required for a DEM task", "errors.bridgeText.demAoiRequired"],
  ["DEM AOI geometry or display identity is stale relative to Store authority", "errors.bridgeText.demAoiStale"],
  ["persisted DEM AOI geometry is invalid", "errors.bridgeText.demAoiGeometryInvalid"],
  ["persisted DEM AOI cannot derive request windows", "errors.bridgeText.demAoiWindowsInvalid"],
]);

// The backend's generic Bhoonidhi reason, compared verbatim; it carries no specific cause.
const BHOONIDHI_GENERIC_AUTH_REASON = "Bhoonidhi 认证不可用"; // i18n-exempt: backend reason_zh literal

/** Localized texts a shell `message_key` is projected to; they carry no cause beyond the code. */
function isShellMessageKeyText(value: string): boolean {
  if (!value) return false;
  return LOCALES.some((locale) =>
    Object.values(MESSAGES[locale].errors.messageKeys).some((group) =>
      Object.values(group as Record<string, string>).includes(value)));
}

function friendlyTextForCode(codeText: string, context: BridgeErrorFormatContext): string | undefined {
  if (codeText === "ASF_QUERY_INVALID" && isBurstDataset(context.dataset)) {
    return t("errors.bridgeText.asfQueryInvalidBurst");
  }
  return lookupErrorText("bridge", codeText) ?? undefined;
}

const TYPED_OPEN_GEODATA_PREFIX =
  /^(?:GEE_[A-Z0-9_]+|GEM15_[A-Z0-9_]+|INVALID_SA_JSON|INVALID_SERVICE_ACCOUNT_JSON|MISSING_BBOX|INVALID_PROXY_URL|EARTHDATA_[A-Z0-9_]+|OPEN_GEODATA_[A-Z0-9_]+|OPEN_EO_[A-Z0-9_]+|DLR_ENMAP_[A-Z0-9_]+|DLR_EOWEB_[A-Z0-9_]+|CMR_SEARCH_[A-Z0-9_]+|ASF_CMR_[A-Z0-9_]+|BHOONIDHI_SBAND_[A-Z0-9_]+|PAYLOAD_TOO_LARGE|INCOMPLETE_TRANSFER|CREATE_OUTPUT_DIR_FAILED|CREATE_DIR_FAILED|RENAME_FAILED|UNSUPPORTED_OPEN_GEODATA_DATASET|UNSUPPORTED_OPEN_EO_DATASET|READ_DOWNLOADED_GRANULE_FAILED)(?:\s*\([^)\n]{1,160}\))?\s*:/;
const OPEN_GEODATA_INTERNAL_TERM_PATTERN =
  /(ApplicationService|Desktop API|\bTauri\b|\bSidecar\b|\bV3_[A-Z0-9_]+|\bGUI\d+\b|does not expose|not implemented|Task summary|task-view|internal owner)/i;

function sanitizeOpenGeodataContext(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed || !TYPED_OPEN_GEODATA_PREFIX.test(trimmed)) return "";
  const isDlrEnmapError = /^(?:DLR_ENMAP_STAC_HTTP_ERROR|DLR_ENMAP_STAC_SEARCH_FAILED)\b/.test(trimmed);
  const scrubbed = trimmed
    .replace(/-----BEGIN [^-]+-----[\s\S]*?-----END [^-]+-----/gi, "[REDACTED_KEY]")
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]+/gi, "Bearer [REDACTED]")
    .replace(/\bya29\.[A-Za-z0-9._-]+/g, "[REDACTED_TOKEN]")
    .replace(
      /\b(?:access_token|refresh_token|client_secret|private_key|password|secret|token|authorization|api_key|key)\s*[:=]\s*["']?[^\s"',;)]+["']?/gi,
      "[REDACTED]",
    )
    .replace(/\(\s*https?:\/\/[^\s)]+\s*\)/gi, "")
    .replace(/https?:\/\/\S+/gi, "")
    .replace(/[A-Za-z]:[\\/]\S+/g, "")
    .replace(/\/(?:Users|home|private|var|tmp)\/\S+/g, "")
    .replace(/([A-Za-z0-9])[A-Za-z0-9._+-]*(@[A-Za-z0-9.-]+\.[A-Za-z]{2,})/g, "$1***$2")
    .replace(/[{}]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!scrubbed || OPEN_GEODATA_INTERNAL_TERM_PATTERN.test(scrubbed)) return "";
  const decorated = isDlrEnmapError
    ? `${scrubbed}（DLR EOC 公开 STAC 目录服务暂时不可用；若已确认景号，请前往 eoweb.dlr.de 登录下载）`
    : scrubbed;
  return decorated.length > 260 ? `${decorated.slice(0, 259).trimEnd()}…` : decorated;
}

export function formatBridgeErrorValue(
  error: unknown,
  context: BridgeErrorFormatContext = {},
): string {
  const sourceLocale = getCurrentLocale() === DEFAULT_LOCALE;
  if (error && typeof error === "object") {
    const value = error as Record<string, unknown>;
    const message = value.error ?? value.message;
    const code = value.code;
    const codeText = code == null ? "" : String(code);
    const contextValue = typeof value.context === "string" ? value.context.trim() : "";
    if (codeText === "V3_OPEN_GEODATA_QUERY_FAILED") {
      const geodataDetail = sanitizeOpenGeodataContext(
        contextValue || (typeof message === "string" ? message : ""),
      );
      if (geodataDetail) return geodataDetail;
    }
    // Only a numeric HTTP status from the typed Bhoonidhi error is exposed.
    // Never render arbitrary provider debug_context, URL or response text.
    if (codeText === "BHOONIDHI_HTTP_ERROR" && /^http_status:[345]\d{2}$/.test(contextValue)) {
      const status = Number(contextValue.slice("http_status:".length));
      const hint = status === 413
        ? t("errors.bridgeText.bhoonidhiHintTooLarge")
        : status === 406
          ? "单次检索时间跨度超过 Bhoonidhi 366 天上限，请缩小日期范围或升级后重试。"
          : status === 400 || status === 422
            ? t("errors.bridgeText.bhoonidhiHintRejectedQuery")
            : t("errors.bridgeText.bhoonidhiHintRetry");
      return t("errors.bridgeText.bhoonidhiHttp", { status, hint });
    }
    const demRange = codeText === "DEM_RANGE_LIMIT_EXCEEDED"
      ? /^width=(\d{1,9});height=(\d{1,9});pixels=(\d{1,18});coarser=([01])$/.exec(contextValue)
      : null;
    if (demRange) {
      return demRangeLimitMessage(Number(demRange[1]), Number(demRange[2]), Number(demRange[3]), demRange[4] === "1");
    }
    // Translate only exact, source-owned reasons. Never interpolate a raw diagnostic here.
    if (codeText === "V3_APPLICATION_INVALID_REQUEST") {
      const demReason = DEM_INVALID_REQUEST_REASONS.get(contextValue);
      if (demReason) return t(demReason);
    }
    if (codeText === "V3_STORE_READINESS_UNSUPPORTED" && contextValue === "formal_dem_download") {
      return t("errors.bridgeText.demNativeGisUnavailable");
    }
    if (codeText === "V3_STORE_READINESS_UNSUPPORTED" && contextValue === "formal_dem_credential") {
      return t("errors.bridgeText.demCredentialUnavailable");
    }
    const contextContainsInternalTerm = containsInternalContextTerm(contextValue);
    const safeContext = contextValue.length > 0
      && contextValue.length <= 240
      && /^[\p{L}\p{N}\s_.,:;()+=><\-，。；：！？、（）]+$/u.test(contextValue)
      && !contextContainsInternalTerm
      && !/(?:password|token|secret|authorization)/i.test(contextValue)
      && !contextValue.includes("://")
      && !/[A-Za-z]:[\\/]/.test(contextValue)
      ? contextValue
      : "";
function formatAsfQueryInvalidDetail(contextValue: string): string | undefined {
  const trimmed = contextValue.trim();
  if (!trimmed) return undefined;
  const resolveFieldLabel = (rawField: string): string => {
    if (rawField === "Path / 轨道号") return "Path";
    if (rawField === "Frame / 帧号") return "Frame";
    return rawField === "检索参数" ? t("errors.bridgeText.generic") : rawField;
  };
  const widthMatch = trimmed.match(/^(Path \/ 轨道号|Frame \/ 帧号|检索参数)需为\s+(\d+)\s+位数字/);
  if (widthMatch) {
    return t("errors.bridgeText.asfQueryInvalidWidth", {
      field: resolveFieldLabel(widthMatch[1]),
      width: widthMatch[2],
    });
  }
  const rangeMatch = trimmed.match(/^(Path \/ 轨道号|Frame \/ 帧号|检索参数)超出合法范围（允许\s+(\d+)[–\-](\d+)）/);
  if (rangeMatch) {
    return t("errors.bridgeText.asfQueryInvalidRange", {
      field: resolveFieldLabel(rangeMatch[1]),
      min: rangeMatch[2],
      max: rangeMatch[3],
    });
  }
  const minMatch = trimmed.match(/^(Path \/ 轨道号|Frame \/ 帧号|检索参数)超出合法范围（需不小于\s+(\d+)）/);
  if (minMatch) {
    return t("errors.bridgeText.asfQueryInvalidRange", {
      field: resolveFieldLabel(minMatch[1]),
      min: minMatch[2],
      max: "∞",
    });
  }
  const maxMatch = trimmed.match(/^(Path \/ 轨道号|Frame \/ 帧号|检索参数)超出合法范围（需不超过\s+(\d+)）/);
  if (maxMatch) {
    return t("errors.bridgeText.asfQueryInvalidRange", {
      field: resolveFieldLabel(maxMatch[1]),
      min: "0",
      max: maxMatch[2],
    });
  }
  return undefined;
}

    const friendly = codeText ? friendlyTextForCode(codeText, context) : undefined;
    if (codeText === "ASF_QUERY_INVALID" && contextValue) {
      const asfDetail = formatAsfQueryInvalidDetail(contextValue);
      if (asfDetail) return asfDetail;
    }
    // In en/ja a Chinese backend detail gives way to the code-keyed explanation when one exists.
    const showChineseDetail = sourceLocale || !friendly;
    if (codeText === "BHOONIDHI_AUTH_UNAVAILABLE" && safeContext
      && CJK_TEXT.test(safeContext) && safeContext !== BHOONIDHI_GENERIC_AUTH_REASON && showChineseDetail) {
      return t("errors.bridgeText.bhoonidhiAuthDetail", { detail: safeContext.replace(/[。；]+$/, "") });
    }
    if (
      codeText === "BHOONIDHI_QUERY_INVALID"
      && safeContext
      && CJK_TEXT.test(safeContext)
      && safeContext !== "Bhoonidhi S-RSLC 目录查询无效"
      && safeContext !== "检索失败，请查看错误码与诊断"
      && showChineseDetail
    ) {
      return `${safeContext.replace(/[。；]+$/, "")}。`;
    }
    const showDetail = safeContext && (showChineseDetail || !CJK_TEXT.test(safeContext));
    if (codeText === "V3_APPLICATION_INVALID_REQUEST" && showDetail) {
      return t("errors.bridgeText.invalidRequestDetail", { detail: safeContext });
    }
    if (codeText === "V3_PROVIDER_OPERATION_FAILED" && showDetail) {
      return t("errors.bridgeText.providerOperationDetail", { detail: safeContext });
    }
    const alwaysFriendlyCodes = new Set([
      "V3_SIDECAR_RESTART_LIMIT",
      "V3_SIDECAR_TIMEOUT",
      "ASF_QUERY_INVALID",
    ]);
    const textMessage =
      message == null
        ? ""
        : String(message)
            .replace(/^Error:\s*/i, "")
            .replace(/\s+\([A-Z0-9_]+\)$/, "")
            .trim();
    // A projected shell message_key is generic, so it is weighed like the raw key it replaces.
    const containsChinese = CJK_TEXT.test(textMessage) && !isShellMessageKeyText(textMessage);
    const containsInternalTerm = containsInternalErrorTerm(textMessage);
    const containsSensitiveDetail =
      /(?:[A-Za-z]:[\\/]|(?:token|password|secret|authorization)\s*[:=]|https?:\/\/\S*[?&](?:token|key|secret)=)/i.test(
        textMessage,
      );
    if (
      friendly
      && (
        !sourceLocale
        || alwaysFriendlyCodes.has(codeText)
        || (codeText === "BHOONIDHI_AUTH_UNAVAILABLE" && textMessage === BHOONIDHI_GENERIC_AUTH_REASON)
        || !containsChinese
        || containsInternalTerm
        || containsSensitiveDetail
      )
    ) return friendly;
    if (containsChinese && !containsInternalTerm && !containsSensitiveDetail) return textMessage;
    if (friendly) return friendly;
  }
  const raw = error instanceof Error ? error.message : String(error ?? "");
  const text = raw
    .replace(/^Error:\s*/i, "")
    .replace(/\s+\([A-Z0-9_]+\)$/, "")
    .trim();
  if (TYPED_OPEN_GEODATA_PREFIX.test(text)) {
    const geodataDetail = sanitizeOpenGeodataContext(text);
    if (geodataDetail) return geodataDetail;
  }
  if (text.includes("NoneType") && text.includes("model_dump")) {
    return t("errors.bridgeText.footprintMissing");
  }
  if (text.includes("model_dump")) {
    return t("errors.bridgeText.dataReadFailed");
  }
  return t("errors.bridgeText.generic");
}
