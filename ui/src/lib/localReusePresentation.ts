import { getCurrentLocale, t } from "../i18n/index.ts";
import { MESSAGES } from "../i18n/locales.ts";
import type { TaskItemLocalReuseProjection } from "./coreRestoreProjection.ts";

/** Downloader D3: how one task item's local reuse decision is shown. */
export type LocalReusePresentation = {
  label: string;
  detail: string | null;
  /** Only a reuse without content evidence offers 重新下载 (Owner 2026-10-20). */
  offerReacquire: boolean;
};

export function presentLocalReuse(
  decision: TaskItemLocalReuseProjection | null | undefined,
  status: string,
): LocalReusePresentation | null {
  if (!decision) return null;
  const locale = getCurrentLocale();
  const proofs = MESSAGES[locale].shared.localReuse.proofs;
  const reasons = MESSAGES[locale].shared.localReuse.reasons as Readonly<Record<string, string>>;
  switch (decision.disposition) {
    case "reused": {
      const grade = decision.grade ?? "size_and_format";
      return { label: proofs[grade], detail: null, offerReacquire: grade === "size_and_format" };
    }
    case "renamed_conflict": {
      const kept = decision.conflict_file_name
        ? t("shared.localReuse.keptAsFile", { fileName: decision.conflict_file_name })
        : t("shared.localReuse.keptDefault");
      return {
        label: status === "completed" ? t("shared.localReuse.completedWithKept", { kept }) : kept,
        detail: decision.conflict_reason ? reasons[decision.conflict_reason] ?? null : null,
        offerReacquire: false,
      };
    }
    case "reacquire_requested":
      return { label: t("shared.localReuse.willRedownloadAndKeep"), detail: null, offerReacquire: false };
  }
}

/** The confirmation of 重新下载: the original is kept, the user decides. */
export function reacquireConfirmationText(displayName: string): string {
  return t("shared.localReuse.reacquireConfirmPrompt", { displayName });
}
