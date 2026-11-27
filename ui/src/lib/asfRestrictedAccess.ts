import { t } from "../i18n/index.ts";

export const ASF_DATASET_ACCESS_RESTRICTED = "ASF_DATASET_ACCESS_RESTRICTED";

export type AsfRestrictedAccessPresentation = {
  title: string;
  badge: string;
  message: string;
  supportLabel: string;
  supportUrl: string;
};

const JERS_RESTRICTED_ACCESS: AsfRestrictedAccessPresentation = {
  get title() { return t("shared.asfRestricted.title"); },
  get badge() { return t("shared.asfRestricted.badge"); },
  get message() { return t("shared.asfRestricted.message"); },
  get supportLabel() { return t("shared.asfRestricted.supportLabel"); },
  supportUrl: "https://asf.alaska.edu/asf/contact/",
};

export function asfRestrictedAccessPresentation(
  datasetId: string | null | undefined,
  failureCode: string | null | undefined,
): AsfRestrictedAccessPresentation | null {
  return datasetId?.trim().toUpperCase() === "JERS-1"
    && failureCode === ASF_DATASET_ACCESS_RESTRICTED
    ? JERS_RESTRICTED_ACCESS
    : null;
}

export function asfRestrictedAccessNoticeKey(
  taskId: string,
  revision: number,
  failureCode: string,
): string {
  return `${taskId}:${revision}:${failureCode}`;
}
