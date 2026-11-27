import { t } from "../../i18n/index.ts";

export function withCreatePending(
  prev: Set<string>,
  itemId: string
): Set<string> {
  const next = new Set(prev);
  next.add(itemId);
  return next;
}

export function withoutCreatePending(
  prev: Set<string>,
  itemId: string
): Set<string> {
  const next = new Set(prev);
  next.delete(itemId);
  return next;
}

export function createTaskButtonLabel(
  isPending: boolean,
  isBatch?: boolean
): string {
  if (isPending) {
    return t("shared.creatingFeedback.verifyingAccount");
  }
  return isBatch ? t("shared.creatingFeedback.enqueueBatch") : t("shared.creatingFeedback.downloadSingle");
}
