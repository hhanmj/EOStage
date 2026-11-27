import { t } from "../i18n/index.ts";

export const DEFAULT_OPTICAL_SEARCH_LIMIT = 100;

export type OpticalDateRangeResult =
  | { ok: true; datetime: { start: string; end: string } | null }
  | { ok: false; error: string };

function normalizedUtc(value: string): string | null {
  const trimmed = value.trim();
  const candidate = /^\d{4}-\d{2}-\d{2}$/.test(trimmed)
    ? `${trimmed}T00:00:00Z`
    : trimmed;
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/.exec(candidate);
  if (!match) return null;
  const instant = new Date(candidate);
  if (!Number.isFinite(instant.getTime())) return null;
  const [year, month, day, hour, minute, second] = match.slice(1).map(Number);
  return instant.getUTCFullYear() === year
    && instant.getUTCMonth() + 1 === month
    && instant.getUTCDate() === day
    && instant.getUTCHours() === hour
    && instant.getUTCMinutes() === minute
    && instant.getUTCSeconds() === second
    ? candidate
    : null;
}

function utcStrictlyBefore(start: string, end: string): boolean {
  const parse = (value: string): [string, string] | null => {
    const match = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,9}))?Z$/.exec(value);
    return match ? [match[1], (match[2] ?? "").padEnd(9, "0")] : null;
  };
  const left = parse(start);
  const right = parse(end);
  return Boolean(left && right && (
    left[0] < right[0] || (left[0] === right[0] && left[1] < right[1])
  ));
}

/** Shared optical semantics: both empty means all time; one-sided ranges are rejected. */
export function buildOptionalOpticalDateRange(startValue: string, endValue: string): OpticalDateRangeResult {
  const rawStart = startValue.trim();
  const rawEnd = endValue.trim();
  if (!rawStart && !rawEnd) return { ok: true, datetime: null };
  if (!rawStart || !rawEnd) {
    return { ok: false, error: t("shared.opticalCatalog.bothDatesOrEmpty") };
  }
  const start = normalizedUtc(rawStart);
  const end = normalizedUtc(rawEnd);
  if (!start || !end) {
    return { ok: false, error: t("shared.opticalCatalog.invalidDateExample") };
  }
  if (!utcStrictlyBefore(start, end)) {
    return { ok: false, error: t("shared.opticalCatalog.endAfterStart") };
  }
  return { ok: true, datetime: { start, end } };
}
