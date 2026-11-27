import { useI18n } from "@/i18n";
import { useId } from "react";
import { Input } from "@/components/ui/input";
import type { DesktopCredentialField, DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";
import { ProviderCredentialField } from "./ProviderCredentialField";

export type SavedProviderCredentials = {
  identity: unknown;
  epoch?: number;
  usernameField: DesktopCredentialField;
  passwordField: DesktopCredentialField;
  onReveal: (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;
};

export interface ProviderAccountFieldsProps {
  provider: string;
  username: string;
  password: string;
  onUsernameChange: (value: string) => void;
  onPasswordChange: (value: string) => void;
  disabled?: boolean;
  passwordLabel?: string;
  passwordCredentialLabel?: string;
  passwordAutoComplete?: string;
  oneTimeCode?: { value: string; onChange: (value: string) => void };
  savedCredentials?: SavedProviderCredentials;
}

/** Controlled fields only: storage and authentication stay in the owning bridge. */
export function ProviderAccountFields({
  provider, username, password, onUsernameChange, onPasswordChange,
  disabled = false, passwordLabel, passwordCredentialLabel,
  passwordAutoComplete = "current-password", oneTimeCode, savedCredentials,
}: ProviderAccountFieldsProps) {
  const { t } = useI18n();
  const resolvedPasswordLabel = passwordLabel ?? t("accounts.password");
  const id = useId();
  const savedUsername = savedCredentials ? {
    identity: savedCredentials.identity,
    epoch: savedCredentials.epoch,
    field: savedCredentials.usernameField,
    onReveal: savedCredentials.onReveal,
  } : undefined;
  const savedPassword = savedCredentials ? {
    identity: savedCredentials.identity,
    epoch: savedCredentials.epoch,
    field: savedCredentials.passwordField,
    onReveal: savedCredentials.onReveal,
  } : undefined;
  return (
    <div className="space-y-2.5" data-provider-account-fields={provider}>
      <div className="grid grid-cols-2 gap-2">
        <ProviderCredentialField label={t("accounts.username")} credentialLabel={t("accounts.fields.credentialLabel", { provider, label: t("accounts.username") })}
          value={username} onChange={onUsernameChange} autoComplete="username" disabled={disabled}
          savedCredential={savedUsername} />
        <ProviderCredentialField label={resolvedPasswordLabel} credentialLabel={passwordCredentialLabel ?? t("accounts.fields.credentialLabel", { provider, label: resolvedPasswordLabel })}
          value={password} onChange={onPasswordChange} secret autoComplete={passwordAutoComplete} disabled={disabled}
          savedCredential={savedPassword} />
      </div>
      {oneTimeCode && (
        <div className="flex items-center gap-3">
          <label htmlFor={`${id}-otp`} className="shrink-0 text-xs text-muted-foreground">{t("accounts.fields.otpLabel")}</label>
          <Input id={`${id}-otp`} aria-label={t("accounts.fields.otpCredentialLabel", { provider })} type="password" inputMode="numeric"
            autoComplete="one-time-code" placeholder={t("accounts.fields.otpPlaceholder")} value={oneTimeCode.value}
            onChange={(event) => oneTimeCode.onChange(event.currentTarget.value)} disabled={disabled}
            className="h-9 min-w-0 rounded-lg" />
        </div>
      )}
    </div>
  );
}
