import { formatByteSize } from "../lib/formatByteSize.ts";
import { getCurrentLocale } from "./index.ts";
import type { Locale } from "./types.ts";

export function formatDate(
  value: Date | number,
  locale: Locale = getCurrentLocale(),
  options?: Intl.DateTimeFormatOptions,
): string {
  return new Intl.DateTimeFormat(locale, options).format(value);
}

export function formatNumber(
  value: number,
  locale: Locale = getCurrentLocale(),
  options?: Intl.NumberFormatOptions,
): string {
  return new Intl.NumberFormat(locale, options).format(value);
}

export function formatCompactNumber(
  value: number,
  locale: Locale = getCurrentLocale(),
  options?: Intl.NumberFormatOptions,
): string {
  return formatNumber(value, locale, { notation: "compact", ...options });
}

export function formatArea(
  value: number,
  locale: Locale = getCurrentLocale(),
  options?: Intl.NumberFormatOptions,
): string {
  return `${formatNumber(value, locale, options)} km²`;
}

export function formatBytes(
  value: number | null | undefined,
  locale: Locale = getCurrentLocale(),
  unknownLabel = "—",
): string {
  const formatted = formatByteSize(value, unknownLabel);
  if (value == null || !Number.isFinite(value) || value < 0) return formatted;
  const [amount, unit] = formatted.split(" ");
  return `${formatNumber(Number(amount), locale, {
    minimumFractionDigits: formatted.split(".")[1]?.split(" ")[0].length ?? 0,
    maximumFractionDigits: 1,
  })} ${unit}`;
}

export function pluralRule(count: number, locale: Locale = getCurrentLocale()): Intl.LDMLPluralRule {
  return new Intl.PluralRules(locale).select(count);
}
