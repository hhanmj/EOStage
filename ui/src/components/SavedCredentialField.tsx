import { useI18n } from "@/i18n";
import { useId } from "react";
import { Eye, EyeOff } from "lucide-react";
import { Input } from "@/components/ui/input";
import type { DesktopCredentialField, DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";
import { useSavedCredentialDisplay } from "./useSavedCredentialDisplay";

/**
 * Saved material is display-only and never copied into an editable draft.
 * Hidden state contains no secret in the DOM: only a placeholder mask is rendered.
 */
export function SavedCredentialField({ label, field, identity, onReveal }: {
  label: string;
  field: DesktopCredentialField;
  identity: unknown;
  onReveal: (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;
}) {
  const { t } = useI18n();
  const id = useId();
  const { shown, pending, value: visibleValue, error, toggle } = useSavedCredentialDisplay({ identity, field, onReveal });
  return <div className="space-y-1 text-xs">
    <label htmlFor={id} className="block font-medium text-muted-foreground">{label} · {t("accounts.fields.saved")}</label>
    <div className="relative">
      <Input id={id} aria-label={t("accounts.fields.savedValue", { label })} type={shown ? "text" : "password"}
        readOnly value={visibleValue} placeholder="••••••••" autoComplete="off" aria-busy={pending}
        aria-describedby={error ? `${id}-error` : undefined}
        className="h-9 rounded-lg pr-9 font-mono" />
      <button type="button" aria-label={t("accounts.fields.revealSaved", { action: shown ? t("accounts.fields.hide") : t("accounts.fields.show"), label })}
        aria-pressed={shown} disabled={false} onClick={() => void toggle()}
        className="absolute right-1 top-1/2 grid h-7 w-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {shown ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
      </button>
    </div>
    {pending && <p className="text-muted-foreground">{t("accounts.fields.reading")}</p>}
    {error && <p id={`${id}-error`} role="alert">{error}</p>}
  </div>;
}
