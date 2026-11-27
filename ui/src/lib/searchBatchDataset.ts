import type { SarSourceCapability } from "./generated/v3-contracts.ts";
import { sarSearchTargetCount } from "./sarSearchStatusPresentation.ts";

export const NISAR_RSLC_LOGICAL_DATASET = "sar.nisar.l.rslc.provisional.v1";

export function isNisarSearchBatchQuery(query: unknown): boolean {
  return Boolean(
    query
      && typeof query === "object"
      && !Array.isArray(query)
      && (query as Record<string, unknown>).logical_dataset === NISAR_RSLC_LOGICAL_DATASET,
  );
}

/** What a restored SearchBatch query can tell the search form. */
export type RestoredSearchFormSeed = {
  dataset: string;
  start: string;
  end: string;
  /** A present date the form cannot show; both dates then keep the form default. */
  datesInvalid: boolean;
  targetCount: number | null;
};

/** The calendar day of a restored date, "" when absent, null when the form cannot show it. */
function restoredCalendarDate(value: unknown): string | null {
  if (value === null || value === undefined) return "";
  if (typeof value !== "string") return null;
  const text = value.trim();
  if (!text) return "";
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(text);
  if (!match) return null;
  const [year, month, day] = match.slice(1).map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
    ? match[0]
    : null;
}

export function restoredSearchFormSeed(query: unknown): RestoredSearchFormSeed | null {
  if (!query || typeof query !== "object" || Array.isArray(query)) return null;
  const record = query as Record<string, unknown>;
  const dataset = record.dataset ?? record.logical_dataset;
  if (typeof dataset !== "string" || !dataset.trim()) return null;
  const targetCount = sarSearchTargetCount(record);
  const start = restoredCalendarDate(record.start);
  const end = restoredCalendarDate(record.end);
  if (start === null || end === null) {
    return { dataset, start: "", end: "", datesInvalid: true, targetCount };
  }
  return { dataset, start, end, datesInvalid: false, targetCount };
}

/** Legacy batches name the source selector; typed catalogue batches name its logical dataset. */
export function resolveRestoredSarSource(
  dataset: string,
  sources: readonly SarSourceCapability[],
): SarSourceCapability | null {
  return sources.find((source) => source.selector === dataset)
    ?? sources.find((source) => source.logical_dataset === dataset)
    ?? null;
}
