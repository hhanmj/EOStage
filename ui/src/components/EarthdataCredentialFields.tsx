import { useI18n } from "@/i18n";
import type { DesktopCredentialField, DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";
import { ProviderAccountFields } from "./ProviderAccountFields";
import { ProviderCredentialField } from "./ProviderCredentialField";
import { cn } from "@/lib/utils";

export type EarthdataCredentialMode = "token" | "login";

export function EarthdataCredentialModeTabs({
  value,
  onChange,
  disabled = false,
}: {
  value: EarthdataCredentialMode;
  onChange: (mode: EarthdataCredentialMode) => void;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  return (
    <div
      role="tablist"
      aria-label={t("accounts.earthdata.modeTabs")}
      className="grid grid-cols-2 gap-1 rounded-xl border border-white/45 bg-white/35 p-1 text-sm dark:border-white/10 dark:bg-white/10"
      data-earthdata-credential-mode-tabs
    >
      {(["token", "login"] as const).map((mode) => {
        const selected = value === mode;
        return (
          <button
            key={mode}
            type="button"
            role="tab"
            aria-selected={selected}
            disabled={disabled}
            onClick={() => onChange(mode)}
            className={cn(
              "h-7 rounded-lg transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
              selected
                ? "bg-primary text-primary-foreground shadow-sm"
                : "text-muted-foreground hover:bg-white/60",
            )}
          >
            {mode === "token" ? "Token" : t("accounts.earthdata.loginTab")}
          </button>
        );
      })}
    </div>
  );
}

export type EarthdataCredentialFieldsProps = {
  mode?: EarthdataCredentialMode | "both";
  status: string | null;
  credentialIdentity: unknown;
  token: string;
  username: string;
  password: string;
  onTokenChange: (value: string) => void;
  onUsernameChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  onReveal?: (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;
  disabled?: boolean;
  busy?: "token" | "login" | null;
};

/** Presentation only: parent state and the named credential bridge own all writes. */
export function EarthdataCredentialFields({ mode = "both", status, credentialIdentity,
  token, username, password, onTokenChange, onUsernameChange, onPasswordChange,
  onReveal, disabled = false, busy = null,
}: EarthdataCredentialFieldsProps) {
  const { t } = useI18n();
  const locked = disabled || busy !== null;
  const canReveal = Boolean(onReveal && !locked);
  const sourceIsExternal = status === "env-token" || status === "netrc";
  return <div className="space-y-2.5" data-earthdata-credential-fields>
    {mode !== "login" && (
      <ProviderCredentialField label="Earthdata Token" value={token} onChange={onTokenChange}
        secret placeholder={t("accounts.earthdata.tokenPlaceholder")} disabled={locked}
        savedCredential={canReveal && status === "token" ? {
          identity: credentialIdentity, field: "earthdata_token", onReveal: onReveal!,
        } : undefined} />
    )}
    {mode !== "token" && (
      <ProviderAccountFields provider="Earthdata" username={username} password={password}
        onUsernameChange={onUsernameChange} onPasswordChange={onPasswordChange} disabled={locked}
        savedCredentials={canReveal && status?.startsWith("login:") ? {
          identity: credentialIdentity, usernameField: "earthdata_username",
          passwordField: "earthdata_password", onReveal: onReveal!,
        } : undefined} />
    )}
    {sourceIsExternal && <p className="text-xs text-muted-foreground">
      {t("accounts.earthdata.source", { source: status === "netrc" ? ".netrc" : t("accounts.earthdata.environment") })}
    </p>}
  </div>;
}
