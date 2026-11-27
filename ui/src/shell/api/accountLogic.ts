import { t } from "../../i18n/index.ts";
import type { AccountId, AccountState } from "./types.ts";

export interface AccountMeta {
  id: AccountId;
  name_zh: string;
  description_zh: string;
  canLogin: boolean;
  canClear: boolean;
  note_zh?: string;
}

/** Descriptions are getters so they follow the active locale at read time. */
export const ACCOUNT_METAS: AccountMeta[] = [
  {
    id: "cdse",
    name_zh: "Copernicus Data Space (CDSE)",
    get description_zh() { return t("accounts.providers.cdseDescription"); },
    canLogin: true,
    canClear: false,
  },
  {
    id: "earthdata",
    name_zh: "NASA Earthdata (ASF DAAC)",
    get description_zh() { return t("accounts.providers.earthdataDescription"); },
    canLogin: true,
    canClear: true,
  },
  {
    id: "bhoonidhi",
    name_zh: "ISRO Bhoonidhi Portal",
    get description_zh() { return t("accounts.providers.bhoonidhiDescription"); },
    canLogin: true,
    canClear: false,
  },
  {
    id: "usgs_m2m",
    name_zh: "USGS EarthExplorer / M2M",
    get description_zh() { return t("accounts.providers.usgsDescription"); },
    canLogin: true,
    canClear: false,
  },
  {
    id: "mpc",
    name_zh: "Microsoft Planetary Computer (MPC)",
    get description_zh() { return t("accounts.providers.mpcDescription"); },
    canLogin: false,
    canClear: false,
    get note_zh() { return t("accounts.providers.mpcNote"); },
  },
  {
    id: "opentopography",
    name_zh: "OpenTopography",
    get description_zh() { return t("accounts.providers.opentopographyDescription"); },
    canLogin: true,
    canClear: true,
    get note_zh() { return t("accounts.providers.opentopographyNote"); },
  },
];

export function getAccountStateLabel(state: AccountState): string {
  switch (state) {
    case "logged_in":
      return t("accounts.valid");
    case "login_required":
      return t("accounts.status.unverified");
    case "credentials_missing":
      return t("accounts.status.notConfigured");
    case "service_unavailable":
      return t("accounts.status.unreachable");
    case "not_required":
      return t("accounts.status.notRequired");
    default:
      return t("accounts.status.unknown");
  }
}

export function orderAccountsForSettings<T extends AccountMeta>(accounts: T[]): T[] {
  return [...accounts.filter((account) => account.canLogin), ...accounts.filter((account) => !account.canLogin)];
}

export function filterRenderedAccounts<T extends AccountId | { id: AccountId }>(accounts: T[]): T[] {
  // 严格过滤掉 gacos 等无需在账号管理页呈现为可配置独立卡片的项
  return accounts.filter((item) => (typeof item === "string" ? item !== "gacos" : item.id !== "gacos"));
}

export function getTaskRunningLabel(category?: string): string {
  return category === "dem_convert" ? t("downloads.status.converting") : t("downloads.status.downloading");
}
