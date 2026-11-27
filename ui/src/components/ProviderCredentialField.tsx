import { useI18n } from "@/i18n";
import { useEffect, useId, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import { Input } from "@/components/ui/input";
import { useSavedCredentialDisplay, type SavedCredentialBinding } from "./useSavedCredentialDisplay";
export type { SavedCredentialBinding } from "./useSavedCredentialDisplay";

export type ProviderCredentialFieldProps = {
  label: string;
  credentialLabel?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  secret?: boolean;
  autoComplete?: string;
  inputMode?: "email" | "numeric" | "search" | "tel" | "text" | "url" | "none" | "decimal";
  savedCredential?: SavedCredentialBinding;
  className?: string;
};

/**
 * One credential input owns both the editable draft and the display-only saved value.
 * Saved material is fetched only after an explicit reveal and is never copied into the draft.
 */
export function ProviderCredentialField({
  label,
  credentialLabel = label,
  value,
  onChange,
  placeholder,
  disabled = false,
  secret = false,
  autoComplete = "off",
  inputMode,
  savedCredential,
  className = "h-9 rounded-lg",
}: ProviderCredentialFieldProps) {
  const { t } = useI18n();
  const id = useId();
  const [showDraft, setShowDraft] = useState(false);
  const saved = useSavedCredentialDisplay(savedCredential, !disabled && !value);
  useEffect(() => {
    setShowDraft(false);
  }, [disabled, !value, savedCredential?.identity, savedCredential?.epoch, savedCredential?.field]);
  const usesSavedCredential = Boolean(savedCredential && !value);
  const savedMode = usesSavedCredential && saved.shown;
  const visibleValue = savedMode ? saved.value : value;
  const showToggle = usesSavedCredential || secret;
  const visible = !disabled && (savedMode || (secret && showDraft));

  return (
    <div className="min-w-0 space-y-1">
      <label htmlFor={id} className="block text-xs font-medium text-muted-foreground">{label}</label>
      <div className="relative">
        <Input
          id={id}
          aria-label={credentialLabel}
          type={secret && !visible ? "password" : "text"}
          autoComplete={autoComplete}
          inputMode={inputMode}
          spellCheck={false}
          value={visibleValue}
          readOnly={savedMode}
          aria-busy={saved.pending}
          aria-describedby={saved.error ? `${id}-error` : undefined}
          placeholder={usesSavedCredential ? "••••••••" : placeholder}
          onChange={(event) => {
            saved.hide();
            onChange(event.currentTarget.value);
          }}
          disabled={disabled}
          className={`${className}${showToggle ? " pr-9" : ""}`}
        />
        {showToggle && (
          <button
            type="button"
            aria-label={usesSavedCredential
              ? t("accounts.fields.revealSaved", { action: saved.shown ? t("accounts.fields.hide") : t("accounts.fields.show"), label: credentialLabel })
              : t("accounts.fields.draftVisibility", { action: showDraft ? t("accounts.fields.hide") : t("accounts.fields.show"), label: credentialLabel })}
            aria-pressed={usesSavedCredential ? saved.shown : showDraft}
            title={usesSavedCredential
              ? t("accounts.fields.revealSavedTitle", { action: saved.shown ? t("accounts.fields.hide") : t("accounts.fields.show") })
              : t("accounts.fields.draftVisibilityTitle", { action: showDraft ? t("accounts.fields.hide") : t("accounts.fields.show") })}
            disabled={disabled}
            onClick={() => usesSavedCredential
              ? void saved.toggle()
              : setShowDraft((current) => !current)}
            className="absolute right-1 top-1/2 grid h-7 w-7 -translate-y-1/2 place-items-center rounded-md text-muted-foreground hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
          >
            {(usesSavedCredential ? saved.shown : showDraft)
              ? <EyeOff className="h-4 w-4" />
              : <Eye className="h-4 w-4" />}
          </button>
        )}
      </div>
      {saved.error && <p id={`${id}-error`} role="alert" className="text-[10px] text-destructive">{saved.error}</p>}
    </div>
  );
}
