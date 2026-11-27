import { useI18n } from "@/i18n";
import type { ReactNode } from "react";
import { KeyRound, RotateCcw, Save, Trash2, X } from "lucide-react";

import { CredentialActionBar } from "@/components/CredentialActionBar";
import { ProviderAccountFields } from "@/components/ProviderAccountFields";
import { ProviderCredentialSurface } from "@/components/ProviderCredentialSurface";
import type {
  DesktopCdseSessionStatus,
  DesktopCredentialField,
  DesktopCredentialRevealResult,
  DesktopUsgsM2mSessionStatus,
} from "@/lib/desktopShellBridge";

type Reveal = (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;

type SharedProps = {
  credentialIdentity: unknown;
  saved: boolean;
  busy: string | null;
  error: string | null;
  note: string | null;
  onReveal?: Reveal;
  onSave: () => void;
  onLogin: () => void;
  onRefresh: () => void;
  onClear: () => void;
  onLogout: () => void;
};

function Feedback({ error, note }: { error: string | null; note: string | null }) {
  return (
    <>
      {error && <p role="alert" className="break-words text-xs text-destructive">{error}</p>}
      {note && <p role="status" className="break-words text-xs text-muted-foreground">{note}</p>}
    </>
  );
}

export function CdseCredentialSessionPanel(props: SharedProps & {
  session: DesktopCdseSessionStatus | null;
  username: string;
  password: string;
  totp: string;
  onUsernameChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onTotpChange: (value: string) => void;
  extra?: ReactNode;
}) {
  const { t } = useI18n();
  const draftStarted = Boolean(props.username.trim() || props.password);
  const draftComplete = Boolean(props.username.trim() && props.password);
  const runtimeAvailable = props.session?.authenticated === true;
  return (
    <ProviderCredentialSurface
      provider="cdse"
      saved={props.saved}
      runtimeLabel={runtimeAvailable ? t("accounts.status.sessionAvailable") : props.saved ? t("accounts.status.credentialsSaved") : t("accounts.status.notConfigured")}
      runtimeTone={runtimeAvailable ? "success" : "neutral"}
    >
      <div className="space-y-3" data-provider-session-panel="cdse">
      <ProviderAccountFields
        provider="CDSE"
        username={props.username}
        password={props.password}
        onUsernameChange={props.onUsernameChange}
        onPasswordChange={props.onPasswordChange}
        oneTimeCode={{ value: props.totp, onChange: props.onTotpChange }}
        disabled={props.busy !== null}
        savedCredentials={props.saved && props.onReveal && props.busy === null ? {
          identity: props.credentialIdentity,
          usernameField: "cdse_username",
          passwordField: "cdse_password",
          onReveal: props.onReveal,
        } : undefined}
      />
      {props.extra}
      <CredentialActionBar
        id="cdse"
        actions={[
          { key: "save", label: t("accounts.actions.save"), icon: <Save className="h-3.5 w-3.5" />, busy: props.busy === "save", disabled: props.busy !== null || !draftComplete, onClick: props.onSave },
          { key: "login", label: props.session?.authenticated ? t("accounts.actions.relogin") : t("accounts.login"), icon: <KeyRound className="h-3.5 w-3.5" />, busy: props.busy === "login", disabled: props.busy !== null || (draftStarted ? !draftComplete : !props.saved), onClick: props.onLogin },
          { key: "check", label: t("accounts.actions.refresh"), icon: <RotateCcw className="h-3.5 w-3.5" />, busy: props.busy === "status", disabled: props.busy !== null, onClick: props.onRefresh },
          { key: "clear", label: t("accounts.clearCredentials"), icon: <Trash2 className="h-3.5 w-3.5" />, busy: props.busy === "clear", disabled: props.busy !== null || !props.saved, onClick: props.onClear },
          ...(props.session?.authenticated
            ? [{ key: "logout", label: t("accounts.actions.logout"), icon: <X className="h-3.5 w-3.5" />, busy: props.busy === "logout", disabled: props.busy !== null, onClick: props.onLogout }]
            : []),
        ]}
      />
      <Feedback error={props.error} note={props.note} />
    </div>
    </ProviderCredentialSurface>
  );
}

export function UsgsCredentialSessionPanel(props: SharedProps & {
  session: DesktopUsgsM2mSessionStatus | null;
  username: string;
  applicationToken: string;
  onUsernameChange: (value: string) => void;
  onApplicationTokenChange: (value: string) => void;
}) {
  const { t } = useI18n();
  const draftStarted = Boolean(props.username.trim() || props.applicationToken);
  const draftComplete = Boolean(props.username.trim() && props.applicationToken);
  const runtimeAvailable = props.session?.authenticated === true;
  const runtimeLabel = runtimeAvailable
    ? t("accounts.status.sessionAvailable")
    : props.session?.state === "login_failed"
      ? t("accounts.status.loginFailed")
      : props.saved
        ? t("accounts.status.credentialsSaved")
        : t("accounts.status.notConfigured");
  return (
    <ProviderCredentialSurface
      provider="usgs"
      saved={props.saved}
      runtimeLabel={runtimeLabel}
      runtimeTone={runtimeAvailable ? "success" : props.session?.state === "login_failed" ? "warning" : "neutral"}
    >
      <div className="space-y-3" data-provider-session-panel="usgs">
      <ProviderAccountFields
        provider="USGS"
        username={props.username}
        password={props.applicationToken}
        onUsernameChange={props.onUsernameChange}
        onPasswordChange={props.onApplicationTokenChange}
        passwordLabel="Application Token"
        passwordCredentialLabel="USGS M2M Application Token"
        passwordAutoComplete="off"
        disabled={props.busy !== null}
        savedCredentials={props.saved && props.onReveal && props.busy === null ? {
          identity: props.credentialIdentity,
          usernameField: "usgs_username",
          passwordField: "usgs_application_token",
          onReveal: props.onReveal,
        } : undefined}
      />
      <CredentialActionBar
        id="usgs"
        actions={[
          { key: "save", label: t("accounts.actions.save"), icon: <Save className="h-3.5 w-3.5" />, busy: props.busy === "save", disabled: props.busy !== null || !draftComplete, onClick: props.onSave },
          { key: "login", label: props.session?.authenticated ? t("accounts.actions.relogin") : t("accounts.login"), icon: <KeyRound className="h-3.5 w-3.5" />, busy: props.busy === "login", disabled: props.busy !== null || (draftStarted ? !draftComplete : !props.saved), onClick: props.onLogin },
          { key: "check", label: t("accounts.actions.refresh"), icon: <RotateCcw className="h-3.5 w-3.5" />, busy: props.busy === "status", disabled: props.busy !== null, onClick: props.onRefresh },
          { key: "clear", label: t("accounts.clearCredentials"), icon: <Trash2 className="h-3.5 w-3.5" />, busy: props.busy === "clear", disabled: props.busy !== null || !props.saved, onClick: props.onClear },
          ...(props.session?.authenticated
            ? [{ key: "logout", label: t("accounts.actions.logout"), icon: <X className="h-3.5 w-3.5" />, busy: props.busy === "logout", disabled: props.busy !== null, onClick: props.onLogout }]
            : []),
        ]}
      />
      <Feedback error={props.error} note={props.note} />
    </div>
    </ProviderCredentialSurface>
  );
}
