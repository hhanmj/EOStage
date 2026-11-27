import { t } from "../../i18n/index.ts";

export type StateVariant =
  | "empty"
  | "loading"
  | "error"
  | "not_connected"
  | "login_required";

export interface StateActionMeta {
  label: string;
  actionId?: string;
}

export interface StatusStateMeta {
  variant: StateVariant;
  iconName: string;
  title: string;
  description?: string;
  defaultAction?: StateActionMeta;
}

export function getStatusStateConfig(variant: StateVariant): StatusStateMeta {
  switch (variant) {
    case "empty":
      return {
        variant: "empty",
        iconName: "Inbox",
        title: t("shared.statusState.emptyTitle"),
        description: t("shared.statusState.emptyDesc"),
      };
    case "loading":
      return {
        variant: "loading",
        iconName: "Loader2",
        title: t("shared.statusState.loadingTitle"),
        description: t("shared.statusState.loadingDesc"),
      };
    case "error":
      return {
        variant: "error",
        iconName: "AlertCircle",
        title: t("shared.statusState.errorTitle"),
        description: t("shared.statusState.errorDesc"),
        defaultAction: {
          label: t("shared.statusState.errorRetryLabel"),
          actionId: "retry",
        },
      };
    case "not_connected":
      return {
        variant: "not_connected",
        iconName: "Plug",
        title: t("shared.statusState.unconnectedTitle"),
        description: t("shared.statusState.unconnectedDesc"),
        defaultAction: {
          label: t("shared.statusState.unconnectedActionLabel"),
          actionId: "open_settings",
        },
      };
    case "login_required":
      return {
        variant: "login_required",
        iconName: "Lock",
        title: t("shared.statusState.authRequiredTitle"),
        description: t("shared.statusState.authRequiredDesc"),
        defaultAction: {
          label: t("shared.statusState.authRequiredActionLabel"),
          actionId: "login",
        },
      };
  }
}
