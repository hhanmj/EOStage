import { useI18n } from "@/i18n";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { ExternalLink, KeyRound, RotateCcw, Save, Trash2 } from "lucide-react";
import { CredentialActionBar } from "./CredentialActionBar";
import type { SimpleOk } from "@/lib/bridge";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ProviderAccountFields } from "./ProviderAccountFields";
import { ProviderCredentialSurface } from "./ProviderCredentialSurface";
import type {
  DesktopBhoonidhiAuthResult,
  DesktopBhoonidhiSessionStatus,
  DesktopCredentialField,
  DesktopCredentialRevealResult,
} from "@/lib/desktopShellBridge";

export interface BhoonidhiCredentialPanelProps {
  status: string | null;
  session?: DesktopBhoonidhiSessionStatus | null;
  sessionError?: string | null;
  showStatusHeader?: boolean;
  credentialIdentity?: unknown;
  onSave: (username: string, password: string) => Promise<SimpleOk>;
  onClear: () => Promise<SimpleOk>;
  onChanged: () => Promise<unknown>;
  onCheckAuth?: () => Promise<DesktopBhoonidhiAuthResult>;
  onReveal?: (field: DesktopCredentialField) => Promise<DesktopCredentialRevealResult>;
  onRegister?: () => void;
}

/** Editable drafts stay separate from saved display-only material. Status comes from the credential service. */
export function BhoonidhiCredentialPanel({
  status,
  session = null,
  sessionError = null,
  showStatusHeader = true,
  credentialIdentity,
  onSave,
  onClear,
  onChanged,
  onCheckAuth,
  onReveal,
  onRegister,
}: BhoonidhiCredentialPanelProps) {
  const { t } = useI18n();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<"save" | "clear" | "refresh" | "auth" | null>(null);
  const [revealEpoch, setRevealEpoch] = useState(0);
  const lifecycle = useRef(0);
  useEffect(() => {
    lifecycle.current++;
    setNote(null); setError(null);
    return () => { lifecycle.current++; };
  }, [status, credentialIdentity]);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const inFlight = useRef(false);
  const configured = status !== null && status !== "none" && status !== "unavailable";
  const statusLabel = status === null ? t("accounts.status.loading") : status === "unavailable"
    ? t("accounts.status.serviceUnavailable")
    : session?.authenticated
      ? t("accounts.status.sessionAvailable")
      : configured ? t("accounts.status.savedNotLoggedIn") : t("accounts.status.configure");
  const displayedError = error ?? sessionError;
  const runtimeLabel = session?.authenticated
    ? t("accounts.status.sessionAvailable")
    : session?.refresh_available && session?.expires_in_seconds === 0
      ? t("accounts.status.sessionRefresh")
      : sessionError
        ? t("accounts.status.error")
        : configured
          ? t("accounts.status.notLoggedIn")
          : t("accounts.status.notConfigured");

  async function run(kind: "save" | "clear" | "refresh" | "auth") {
    if (inFlight.current) return;
    if (kind === "save" && (!username.trim() || !password)) return;
    inFlight.current = true;
    const current = lifecycle.current;
    setRevealEpoch((value) => value + 1);
    setBusy(kind);
    setError(null);
    setNote(null);
    try {
      if (kind === "auth") {
        const result = await onCheckAuth?.();
        if (current !== lifecycle.current) return;
        if (result?.ok) {
          await onChanged();
          if (current !== lifecycle.current) return;
          setNote(t("accounts.feedback.loginAvailable"));
        } else setError(result ? formatBridgeErrorValue(result) : t("accounts.feedback.authUnavailable"));
        return;
      }
      if (kind === "save" || kind === "clear") {
        const result = kind === "clear"
          ? await onClear()
          : await onSave(username.trim(), password);
        if (current !== lifecycle.current) return;
        if (!result.ok) {
          setError(formatBridgeErrorValue(result));
          return;
        }
        if (kind === "clear") {
          setUsername("");
          setPassword("");
          setNote(t("accounts.feedback.cleared"));
        } else {
          setUsername("");
          setPassword("");
          setNote(t("accounts.feedback.saved"));
        }
      }
      try {
        await onChanged();
        if (kind === "refresh") setNote(t("accounts.feedback.refreshed"));
      } catch {
        setError(t("accounts.feedback.readFailed"));
      }
    } catch (cause) {
      setError(formatBridgeErrorValue(cause));
    } finally {
      // Never retain a submitted password while waiting for another user action.
      if (kind === "save") setPassword("");
      inFlight.current = false;
      setBusy(null);
    }
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void run("save");
  }

  return (
    <form aria-label={t("accounts.bhoonidhi.settings")} onSubmit={submit} className="h-full">
      {showStatusHeader && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <span className="text-xs text-muted-foreground">{t("accounts.bhoonidhi.purpose")}</span>
          <Badge variant={session?.authenticated ? "success" : configured || status === "unavailable" ? "warning" : "neutral"}>
            {statusLabel}
          </Badge>
        </div>
      )}
      <ProviderCredentialSurface
        provider="bhoonidhi"
        saved={configured}
        runtimeLabel={runtimeLabel}
        runtimeTone={session?.authenticated ? "success" : configured || sessionError ? "warning" : "neutral"}
      >
      <div className="space-y-3">
      <ProviderAccountFields provider="Bhoonidhi" username={username} password={password}
        onUsernameChange={setUsername} onPasswordChange={setPassword} disabled={busy !== null}
        savedCredentials={configured && onReveal && busy === null ? {
          identity: credentialIdentity ?? status,
          epoch: revealEpoch,
          usernameField: "bhoonidhi_username",
          passwordField: "bhoonidhi_password",
          onReveal,
        } : undefined} />
      {onRegister && (
        <Button type="button" variant="outline" size="sm" className="h-8 w-full" onClick={onRegister}>
          <ExternalLink className="h-4 w-4" />
          {t("accounts.actions.register")}
        </Button>
      )}
      <CredentialActionBar
        id="bhoonidhi"
        actions={[
          { key: "save", label: t("accounts.actions.save"), icon: <Save className="h-3.5 w-3.5" />, busy: busy === "save", disabled: busy !== null || !username.trim() || !password, type: "submit" },
          { key: "auth", label: t("accounts.actions.checkLogin"), icon: <KeyRound className="h-3.5 w-3.5" />, busy: busy === "auth", disabled: busy !== null || !configured || !onCheckAuth, onClick: () => void run("auth") },
          { key: "refresh", label: t("accounts.actions.refresh"), icon: <RotateCcw className="h-3.5 w-3.5" />, busy: busy === "refresh", disabled: busy !== null, onClick: () => void run("refresh") },
          { key: "clear", label: t("accounts.clearCredentials"), icon: <Trash2 className="h-3.5 w-3.5" />, busy: busy === "clear", disabled: busy !== null || !configured, onClick: () => void run("clear") },
        ]}
      />
      {displayedError && <p role="alert" className="break-words text-xs text-destructive">{displayedError}</p>}
      {note && <p role="status" className="break-words text-xs text-muted-foreground">{note}</p>}
      </div>
      </ProviderCredentialSurface>
    </form>
  );
}
