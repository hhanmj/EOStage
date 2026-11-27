/**
 * Backend error codes and their user-facing explanations.
 * The text lives in the `errors` i18n namespace keyed by code; an unknown code
 * always yields null and never an invented explanation.
 */
import { DEFAULT_LOCALE, getCurrentLocale } from "../../i18n/index.ts";
import { MESSAGES } from "../../i18n/locales.ts";
import type { Locale } from "../../i18n/types.ts";

type ErrorTable = "explanations" | "bridge";

/** The zh-CN explanation table; kept as the source of truth for which codes are known. */
export const ERROR_CODE_MAP: Readonly<Record<string, string>> = MESSAGES["zh-CN"].errors.explanations;

/**
 * Look up a code-keyed error text in the given locale. Only codes present in the
 * zh-CN source table are known; a locale missing one falls back to the zh-CN text.
 */
export function lookupErrorText(
  table: ErrorTable,
  code: unknown,
  locale: Locale = getCurrentLocale(),
): string | null {
  if (!code || typeof code !== "string") return null;
  const source = MESSAGES["zh-CN"].errors[table] as Readonly<Record<string, string>>;
  if (!Object.prototype.hasOwnProperty.call(source, code)) return null;
  const localized = MESSAGES[locale].errors[table] as Readonly<Record<string, string>>;
  const text = Object.prototype.hasOwnProperty.call(localized, code) ? localized[code] : source[code];
  return text || null;
}

/**
 * The current-locale text for a stable shell `message_key` such as
 * `application_service.unavailable`; null when the key is unknown.
 */
export function explainMessageKey(messageKey: unknown): string | null {
  if (!messageKey || typeof messageKey !== "string") return null;
  const parts = messageKey.split(".");
  if (parts.length !== 2) return null;
  const [group, name] = parts;
  const read = (locale: Locale): string | undefined => {
    const groups = MESSAGES[locale].errors.messageKeys as Readonly<Record<string, Readonly<Record<string, string>>>>;
    if (!Object.prototype.hasOwnProperty.call(groups, group)) return undefined;
    const entries = groups[group];
    return Object.prototype.hasOwnProperty.call(entries, name) ? entries[name] : undefined;
  };
  const source = read("zh-CN");
  if (!source) return null;
  return read(getCurrentLocale()) || source;
}

/**
 * The explanation for an error code in the current locale; null for an unknown or empty code.
 */
export function explainErrorCode(code?: string | null): string | null {
  return lookupErrorText("explanations", code);
}

/**
 * The explanation when one exists, otherwise the original code unchanged.
 */
export function formatErrorWithCode(code: string): string {
  if (!code || typeof code !== "string") return "";
  return explainErrorCode(code) || code;
}

/**
 * Choose between a backend `message_zh` and the code-keyed explanation.
 * zh-CN keeps today's order (the backend sentence first); en/ja prefer the
 * localized explanation and show `message_zh` only when the code is unknown.
 */
export function preferCodeExplanation(
  messageZh: string | null | undefined,
  code: string | null | undefined,
): string | null {
  const message = typeof messageZh === "string" && messageZh ? messageZh : null;
  if (getCurrentLocale() === DEFAULT_LOCALE) return message || explainErrorCode(code);
  return explainErrorCode(code) || message;
}

/**
 * Explain an action error (structured Tauri errors and anything else).
 * When err is an object with a string code that has an explanation, return it;
 * otherwise return the caller's fallback. Never returns err.message, message_key,
 * context or the raw code.
 */
export function explainActionError(err: unknown, fallbackZh: string): string {
  if (err && typeof err === "object" && "code" in err) {
    const code = (err as { code: unknown }).code;
    if (typeof code === "string") {
      const explanation = explainErrorCode(code);
      if (explanation) {
        return explanation;
      }
    }
  }
  return fallbackZh;
}

const DOWNLOAD_CODE_LOCALE_KEYS: Readonly<
  Record<string, "dlCodeCancelled" | "dlCodeIntegrity" | "dlCodeDisk" | "dlCodeAuth" | "dlCodeTransient">
> = {
  DL001: "dlCodeCancelled",
  DL002: "dlCodeIntegrity",
  DL003: "dlCodeDisk",
  DL004: "dlCodeAuth",
  DL005: "dlCodeTransient",
};

/**
 * Localized human-readable summary for download lifecycle codes (DL001–DL005).
 * Kept separate from `errors.explanations` so the frozen i18n parity fixture stays intact.
 */
export function explainDownloadCode(
  code?: string | null,
  locale: Locale = getCurrentLocale(),
): string | null {
  if (!code || typeof code !== "string") return null;
  const normalized = code.trim().toUpperCase();
  const key = DOWNLOAD_CODE_LOCALE_KEYS[normalized];
  if (!key) return null;
  return MESSAGES[locale].mapWorkbench.row[key] || MESSAGES["zh-CN"].mapWorkbench.row[key] || null;
}

export interface DownloadFailureGuidanceInput {
  codes?: readonly string[] | null;
  messageZh?: string | null;
}

export interface DownloadFailurePresentation {
  cause: string | null;
  guidance: string | null;
  extraExplanations: string[];
  diagnosticCodes: string[];
}

/**
 * Project a task failure into a user-first presentation: real sanitized failure cause
 * + targeted actionable guidance upfront, with internal codes demoted to diagnostic refs.
 */
export function describeDownloadFailure(
  input: DownloadFailureGuidanceInput | null | undefined,
  locale: Locale = getCurrentLocale(),
): DownloadFailurePresentation {
  const row = MESSAGES[locale].mapWorkbench.row;
  const rawMessage = typeof input?.messageZh === "string" ? input.messageZh.trim() : "";
  const codes = Array.isArray(input?.codes)
    ? input.codes.filter((c): c is string => typeof c === "string" && c.trim().length > 0)
    : [];
  const upperCodes = codes.map((c) => c.trim().toUpperCase());

  let cause: string | null = rawMessage || null;
  if (!cause && codes.length > 0) {
    cause =
      lookupErrorText("explanations", codes[0], locale) ||
      explainDownloadCode(codes[0], locale) ||
      null;
  }

  const combined = `${rawMessage} ${upperCodes.join(" ")}`.toLowerCase();
  const isStaleProductRejection =
    !upperCodes.includes("DL004") &&
    !upperCodes.some(
      (c) =>
        c.startsWith("BHOONIDHI_AUTH_") ||
        c === "BHOONIDHI_UNAUTHORIZED" ||
        c === "BHOONIDHI_FORBIDDEN" ||
        c === "BHOONIDHI_ACCOUNT_LOGIN_REQUIRED",
    ) &&
    /bhoonidhi/i.test(combined) &&
    /(重新检索|更新编号|重新处理|re-?search|reprocessed|updated product id)/i.test(combined);
  let guidance: string | null = null;
  if (isStaleProductRejection) {
    guidance = null;
  } else if (/tls|ssl|证书|certificate|handshake|握手/.test(combined)) {
    guidance = row.dlGuideTls;
  } else if (/401|403|认证|凭据|登录|会话|token|auth|credential|cookie|dl004/.test(combined)) {
    guidance = row.dlGuideAuth;
  } else if (
    /429|502|503|504|限流|繁忙|rate[\s_-]?limit|too many|service unavailable|bad gateway|gateway timeout|服务暂时不可用|上游服务/.test(
      combined,
    )
  ) {
    guidance = row.dlGuideServiceBusy;
  } else if (/校验|完整性|checksum|sha256|md5|corrupt|dl002/.test(combined)) {
    guidance = row.dlGuideIntegrity;
  } else if (/磁盘|空间不足|不可写|权限|no space|disk_low|dl003|storage_root_not_writable/.test(combined)) {
    guidance = row.dlGuideDisk;
  } else if (/已取消|cancelled|canceled|dl001/.test(combined)) {
    guidance = row.dlGuideCancelled;
  } else if (/超时|timeout|dns|代理|proxy|连接|重置|reset|eof|中断|低速|stall|network|dl005/.test(combined)) {
    guidance = row.dlGuideNetwork;
  }

  if (guidance && rawMessage && rawMessage.includes(guidance)) {
    guidance = null;
  }

  const isBareGenericTransient =
    /^下载连接暂时失败[，,]?\s*已按重试策略处理[。.]?$/.test(rawMessage);

  const extraExplanations: string[] = [];
  for (const code of codes) {
    const knownExplanation = lookupErrorText("explanations", code, locale);
    const dlExplanation =
      !rawMessage || isBareGenericTransient ? explainDownloadCode(code, locale) : null;
    const explained = knownExplanation || dlExplanation;
    if (
      explained &&
      explained !== cause &&
      (!cause || !cause.includes(explained)) &&
      !extraExplanations.includes(explained)
    ) {
      extraExplanations.push(explained);
    }
  }

  return {
    cause,
    guidance,
    extraExplanations,
    diagnosticCodes: codes,
  };
}
