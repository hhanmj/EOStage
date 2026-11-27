import { t } from "../../i18n/index.ts";

export type AccountPortalTarget =
  | "cdse"
  | "cdse_register"
  | "earthdata"
  | "earthdata_register"
  | "earthdata_token"
  | "bhoonidhi"
  | "bhoonidhi_register"
  | "usgs_m2m"
  | "usgs_register"
  | "usgs_token"
  | "gacos"
  | "mpc"
  | "opentopography"
  | "opentopography_key"
  | "gee"
  | "gee_project_id"
  | "gee_service_account"
  | "gee_oauth_token";

export interface AccountPortalLinkSpec {
  target: AccountPortalTarget;
  url: string;
  readonly label: string;
}

/**
 * 权威账号中心与凭据直达地址映射表。
 * 使用 getter 动态读取当前语言文案，保证 zh-CN / en / ja 实时切换一致。
 */
export const ACCOUNT_PORTAL_LINKS: Readonly<Record<AccountPortalTarget, AccountPortalLinkSpec>> = {
  cdse: {
    target: "cdse",
    url: "https://dataspace.copernicus.eu/",
    get label() {
      return t("accounts.portalLinks.cdse");
    },
  },
  cdse_register: {
    target: "cdse_register",
    url: "https://dataspace.copernicus.eu/",
    get label() {
      return t("accounts.portalLinks.cdseRegister");
    },
  },
  earthdata: {
    target: "earthdata",
    url: "https://urs.earthdata.nasa.gov/",
    get label() {
      return t("accounts.portalLinks.earthdata");
    },
  },
  earthdata_register: {
    target: "earthdata_register",
    url: "https://urs.earthdata.nasa.gov/users/new",
    get label() {
      return t("accounts.portalLinks.earthdataRegister");
    },
  },
  earthdata_token: {
    target: "earthdata_token",
    url: "https://urs.earthdata.nasa.gov/profile",
    get label() {
      return t("accounts.portalLinks.earthdataToken");
    },
  },
  bhoonidhi: {
    target: "bhoonidhi",
    url: "https://bhoonidhi.nrsc.gov.in/",
    get label() {
      return t("accounts.portalLinks.bhoonidhi");
    },
  },
  bhoonidhi_register: {
    target: "bhoonidhi_register",
    url: "https://bhoonidhi.nrsc.gov.in/",
    get label() {
      return t("accounts.portalLinks.bhoonidhiRegister");
    },
  },
  usgs_m2m: {
    target: "usgs_m2m",
    url: "https://ers.cr.usgs.gov/",
    get label() {
      return t("accounts.portalLinks.usgs");
    },
  },
  usgs_register: {
    target: "usgs_register",
    url: "https://ers.cr.usgs.gov/register/",
    get label() {
      return t("accounts.portalLinks.usgsRegister");
    },
  },
  usgs_token: {
    target: "usgs_token",
    url: "https://ers.cr.usgs.gov/profile",
    get label() {
      return t("accounts.portalLinks.usgsToken");
    },
  },
  gacos: {
    target: "gacos",
    url: "http://www.gacos.net/",
    get label() {
      return t("accounts.portalLinks.gacos");
    },
  },
  mpc: {
    target: "mpc",
    url: "https://planetarycomputer.microsoft.com/catalog",
    get label() {
      return t("accounts.portalLinks.mpc");
    },
  },
  opentopography: {
    target: "opentopography",
    url: "https://portal.opentopography.org/newUser",
    get label() {
      return t("accounts.portalLinks.opentopography");
    },
  },
  opentopography_key: {
    target: "opentopography_key",
    url: "https://portal.opentopography.org/requestService?service=api",
    get label() {
      return t("accounts.portalLinks.opentopographyKey");
    },
  },
  gee: {
    target: "gee",
    url: "https://code.earthengine.google.com/",
    get label() {
      return t("accounts.portalLinks.gee");
    },
  },
  gee_project_id: {
    target: "gee_project_id",
    url: "https://console.cloud.google.com/earth-engine/configuration",
    get label() {
      return t("accounts.portalLinks.geeProjectId");
    },
  },
  gee_service_account: {
    target: "gee_service_account",
    url: "https://console.cloud.google.com/iam-admin/serviceaccounts",
    get label() {
      return t("accounts.portalLinks.geeServiceAccount");
    },
  },
  gee_oauth_token: {
    target: "gee_oauth_token",
    url: "https://developers.google.com/earth-engine/guides/auth",
    get label() {
      return t("accounts.portalLinks.geeOauthToken");
    },
  },
};

export function resolveAccountPortalLink(
  target: string,
): AccountPortalLinkSpec | null {
  return Object.prototype.hasOwnProperty.call(ACCOUNT_PORTAL_LINKS, target)
    ? ACCOUNT_PORTAL_LINKS[target as AccountPortalTarget]
    : null;
}
