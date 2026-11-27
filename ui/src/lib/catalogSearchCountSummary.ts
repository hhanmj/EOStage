import { t } from "../i18n/index.ts";

export type CatalogSearchCountSummaryInput = {
  targetCount: number | null;
  providerMatchedCount: number | null;
  providerReturnedCount: number | null;
  frozenCandidateCount: number;
  currentPageCount: number;
  checkedCount: number;
  truncated: boolean;
};

export type CatalogSearchCountSummary = {
  target: string | null;
  matched: string | null;
  frozen: string;
  current: string;
  selected: string;
};

export const SAR_ALL_RESULTS_SAFETY_LIMIT = 10_000;

/** The existing possibly-incomplete wording for a frozen batch of `frozenCount`. */
export function possiblyIncompleteLabel(frozenCount: number): string {
  return frozenCount >= SAR_ALL_RESULTS_SAFETY_LIMIT
    ? t("mapWorkbench.results.safetyLimit", { limit: SAR_ALL_RESULTS_SAFETY_LIMIT })
    : t("mapWorkbench.results.incomplete");
}

/** User-facing wording deliberately keeps provider, frozen, page, and selection scopes distinct. */
export function catalogSearchCountSummary(input: CatalogSearchCountSummaryInput): CatalogSearchCountSummary {
  return {
    target: input.targetCount !== null ? t("mapWorkbench.results.targetCount", { target: input.targetCount }) : null,
    matched: input.providerMatchedCount !== null
      ? input.targetCount === null
        && input.truncated
        && input.providerMatchedCount > input.frozenCandidateCount
        ? t("mapWorkbench.results.matchedIncomplete", { matched: input.providerMatchedCount, label: possiblyIncompleteLabel(input.frozenCandidateCount) })
        : t("mapWorkbench.results.matchedCount", { matched: input.providerMatchedCount })
      : input.targetCount !== null
        ? null
        : t("mapWorkbench.results.matchedUnknown"),
    frozen: t("mapWorkbench.results.frozenCount", { loaded: input.frozenCandidateCount }),
    current: t("mapWorkbench.results.currentCount", { shown: input.currentPageCount }),
    selected: t("mapWorkbench.results.selectedCount", { selected: input.checkedCount }),
  };
}
