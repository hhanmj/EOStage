import { useEffect, useRef, useState } from "react";
import { useI18n } from "@/i18n";
import { ChevronDown, ExternalLink, KeyRound, Loader2 } from "lucide-react";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { ProviderAccountFields } from "./ProviderAccountFields";
import { ProviderCredentialField } from "./ProviderCredentialField";
import { EarthdataCredentialModeTabs } from "./EarthdataCredentialFields";
import { getDesktopShellBridge, type DesktopCredentialField } from "@/lib/desktopShellBridge";
import { credentialEditorWriter, type AccountProvider } from "@/lib/credentialEditor";
import { checkAccountLogin, readAccountSession, type AccountSessionObservation } from "@/lib/settingsAccountActions";
import { openExternalUrl } from "@/lib/bridge";

type AccountDefinition = { id: AccountProvider; title: string; fields: DesktopCredentialField[]; register?: string; token?: string };
export const ACCOUNT_DEFINITIONS: readonly AccountDefinition[] = [
  { id: "cdse", title: "CDSE", fields: ["cdse_username", "cdse_password"], register: "https://dataspace.copernicus.eu/" },
  { id: "bhoonidhi", title: "Bhoonidhi", fields: ["bhoonidhi_username", "bhoonidhi_password"], register: "https://bhoonidhi.nrsc.gov.in/" },
  { id: "usgs", title: "USGS", fields: ["usgs_username", "usgs_application_token"], register: "https://ers.cr.usgs.gov/register/", token: "https://ers.cr.usgs.gov/profile" },
  { id: "earthdata", title: "Earthdata / ASF", fields: ["earthdata_username", "earthdata_password", "earthdata_token"], register: "https://urs.earthdata.nasa.gov/users/new", token: "https://urs.earthdata.nasa.gov/" },
  { id: "opentopography", title: "OpenTopography", fields: ["opentopography_key"], register: "https://portal.opentopography.org/newUser", token: "https://portal.opentopography.org/requestService?service=api" },
  { id: "gacos", title: "GACOS", fields: ["gacos_email"] },
];
type Values = Partial<Record<DesktopCredentialField, string>>;

/** Shared by the real Workbench Settings panel and the standalone Settings page.
 * Editable secrets exist only in this mounted form, never status or storage JSON. */
export function SettingsAccounts({ active = true, className = "space-y-3", onChanged }: {
  active?: boolean; className?: string; onChanged?: () => void;
}) {
  return active ? <div className={className} data-settings-accounts>
    {ACCOUNT_DEFINITIONS.map(account => <AccountEditor key={account.id} account={account} onChanged={onChanged} />)}
  </div> : null;
}

function AccountEditor({ account, onChanged }: { account: AccountDefinition; onChanged?: () => void }) {
  const { t } = useI18n();
  const port = getDesktopShellBridge().credentials;
  const writer = port ? credentialEditorWriter(port) : null;
  const [values, setValues] = useState<Values>({});
  const [loadState, setLoadState] = useState<"loading" | "ready" | "failed">("loading");
  const [mode, setMode] = useState<"login" | "token">("login");
  const [source, setSource] = useState<string>("none");
  const [saving, setSaving] = useState(new Set<DesktopCredentialField>());
  const [errors, setErrors] = useState<Partial<Record<DesktopCredentialField, string>>>({});
  const [priorSaveFailures, setPriorSaveFailures] = useState<DesktopCredentialField[]>([]);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [session, setSession] = useState<AccountSessionObservation>({ state: "unknown" });
  const [checking, setChecking] = useState(false);
  const [totp, setTotp] = useState("");
  const [loadAttempt, setLoadAttempt] = useState(0);
  const lifecycle = useRef(0), editEpoch = useRef(0), fieldEpoch = useRef(new Map<DesktopCredentialField, number>());
  const statusInFlight = useRef(false), checkInFlight = useRef(false);
  const changed = useRef(onChanged); changed.current = onChanged;

  useEffect(() => {
    const generation = ++lifecycle.current;
    setLoadState("loading"); setValues({}); setSession({ state: "unknown" });
    let stopped = false;
    const refresh = async () => {
      if (!port || !writer || stopped || statusInFlight.current || checkInFlight.current) return;
      statusInFlight.current = true;
      const epoch = editEpoch.current;
      try {
        const [status, observation] = await Promise.all([port.getStatus(), readAccountSession(account.id, port)]);
        if (stopped || epoch !== editEpoch.current || generation !== lifecycle.current) return;
        setSource(status.ok ? status.status[account.id] : "unavailable");
        const sourceAvailable = status.ok && status.status[account.id] !== "unavailable";
        const checked = sourceAvailable ? writer.checkedState(account.id, status.status[account.id]) : null;
        // Earthdata exposes a check result, not a persistent session. Keep that
        // result for the same editor revision/source instead of erasing it on focus.
        setSession(!sourceAvailable || observation.state === "unavailable" ? { state: "unavailable" }
          : (account.id === "earthdata" && checked) || writer.needsCheck(account.id)
            ? { state: checked ?? "unknown" } : observation);
      } catch {
        if (!stopped && epoch === editEpoch.current && generation === lifecycle.current) setSession({ state: "unavailable" });
      } finally { statusInFlight.current = false; }
    };
    void (async () => {
      if (!port || !writer) { setLoadState("failed"); return; }
      try {
        const saved = await writer.read(account.fields);
        if (stopped || generation !== lifecycle.current) return;
        setValues(saved);
        setPriorSaveFailures(writer.failedFields(account.fields));
        if (account.id === "earthdata") setMode(saved.earthdata_token ? "token" : "login");
        setLoadState("ready"); await refresh();
      } catch { if (!stopped) setLoadState("failed"); }
    })();
    const focus = () => { void refresh(); };
    const visibility = () => { if (document.visibilityState === "visible") focus(); };
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    const interval = window.setInterval(() => { if (document.visibilityState !== "hidden") focus(); }, 30_000);
    return () => {
      stopped = true; lifecycle.current++; editEpoch.current++;
      window.clearInterval(interval); window.removeEventListener("focus", focus);
      document.removeEventListener("visibilitychange", visibility);
      // Accepted writes finish in the transport sequencer; no draft is persisted elsewhere.
    };
  }, [account, port, writer, loadAttempt]);

  function edit(field: DesktopCredentialField, value: string) {
    setValues(current => ({ ...current, [field]: value }));
    // Do not reinterpret yesterday's local-login presence as an external source
    // after the user empties the local fields. Passive refresh can observe it anew.
    setSource("none");
    const clearedFields: DesktopCredentialField[] = account.id === "earthdata" && value
      ? field === "earthdata_token" ? ["earthdata_username", "earthdata_password"] : ["earthdata_token"] : [];
    const clearEpochs = clearedFields.map(other => [other, fieldEpoch.current.get(other) ?? 0] as const);
    setFeedback(null); setSession({ state: "unknown" }); editEpoch.current++;
    const fieldGeneration = (fieldEpoch.current.get(field) ?? 0) + 1; fieldEpoch.current.set(field, fieldGeneration);
    const generation = lifecycle.current;
    setSaving(current => new Set(current).add(field));
    void writer?.write(field, value).then(result => {
      if (generation !== lifecycle.current || fieldEpoch.current.get(field) !== fieldGeneration) return;
      setSaving(current => { const next = new Set(current); next.delete(field); return next; });
      setErrors(current => ({ ...current, [field]: result.ok ? undefined : result.error }));
      if (result.ok && !result.superseded) {
        // The native store rolls a failed mode switch back. Mirror destructive
        // side effects only after acknowledgement, and never erase a newer edit.
        const acknowledgedClears = clearEpochs.filter(([other, epoch]) => (fieldEpoch.current.get(other) ?? 0) === epoch);
        setPriorSaveFailures(current => current.filter(failed => failed !== field && !acknowledgedClears.some(([other]) => other === failed)));
        if (acknowledgedClears.length) {
          setValues(current => { const next = { ...current }; for (const [other] of acknowledgedClears) next[other] = ""; return next; });
          setErrors(current => { const next = { ...current }; for (const [other] of acknowledgedClears) delete next[other]; return next; });
        }
        changed.current?.();
      }
    });
  }
  const userField = account.fields[0]!, passwordField = account.fields[1];
  const complete = account.id === "earthdata" && mode === "token" ? Boolean(values.earthdata_token?.trim())
    : passwordField ? Boolean(values[userField]?.trim() && values[passwordField]) : Boolean(values[userField]?.trim());
  const hasContent = account.fields.some(field => Boolean(values[field]));
  const external = account.id === "earthdata" && !hasContent
    && (source === "netrc" || source === "env-token" || source.startsWith("login:"));
  const canCheck = ["cdse", "usgs", "bhoonidhi", "earthdata"].includes(account.id);
  const errorMessages = Object.values(errors).filter(Boolean);
  const badge = loadState === "loading" ? t("settings.accounts.loading") : loadState === "failed" ? t("settings.accounts.loadFailed")
    : saving.size ? t("settings.accounts.saving") : errorMessages.length || priorSaveFailures.length ? t("settings.accounts.unsaved") : checking ? t("settings.accounts.checking")
      : !hasContent && !external ? t("settings.accounts.notConfigured") : !complete && !external ? t("settings.accounts.incomplete")
        : session.state === "unavailable" ? t("settings.accounts.unavailable") : session.state === "valid" ? account.id === "earthdata" ? t("settings.accounts.earthdataValid") : t("settings.accounts.valid")
          : session.state === "invalid" ? t("settings.accounts.invalid") : canCheck ? t("settings.accounts.unchecked") : t("settings.accounts.saved");
  async function check() {
    if (!port || !writer || checkInFlight.current || saving.size || errorMessages.length || priorSaveFailures.length || (!complete && !external)) return;
    checkInFlight.current = true; setChecking(true); setFeedback(null);
    const generation = lifecycle.current, epoch = ++editEpoch.current, revision = writer.revision(account.id);
    try {
      await writer.settled();
      const status = await port.getStatus();
      const checkedSource = status.ok ? status.status[account.id] : undefined;
      const result = await checkAccountLogin(account.id, port, totp);
      if (generation !== lifecycle.current || epoch !== editEpoch.current) return;
      writer.checked(account.id, revision, result.state, checkedSource);
      if (checkedSource !== undefined) setSource(checkedSource);
      setSession(result); setFeedback(result.message ?? null);
      changed.current?.();
    } catch {
      if (generation === lifecycle.current && epoch === editEpoch.current) {
        writer.checked(account.id, revision, "unavailable", source); setSession({ state: "unavailable" }); setFeedback(t("settings.accounts.checkFailed"));
      }
    } finally {
      checkInFlight.current = false;
      if (generation === lifecycle.current) { setChecking(false); setTotp(""); }
    }
  }
  async function open(url: string) {
    try { const result = await openExternalUrl(url); if (!result.ok) setFeedback(t("settings.accounts.externalOpenFailed")); }
    catch { setFeedback(t("settings.accounts.externalOpenFailed")); }
  }
  return <details open className="min-w-0 rounded-xl border bg-card text-card-foreground" data-account-provider={account.id}>
    <summary className="flex cursor-pointer list-none items-center justify-between gap-2 px-3 py-2.5 text-sm font-medium">
      <span className="flex min-w-0 items-center gap-2"><ChevronDown className="h-3.5 w-3.5 shrink-0" />{account.title}</span>
      <Badge variant={[t("settings.accounts.valid"), t("settings.accounts.earthdataValid")].includes(badge) ? "success" : [t("settings.accounts.loadFailed"), t("settings.accounts.unsaved"), t("settings.accounts.invalid"), t("settings.accounts.unavailable")].includes(badge) ? "warning" : "neutral"} data-account-status>{badge}</Badge>
    </summary>
    <div className="space-y-3 px-3 pb-3">
      {account.id === "earthdata" && <EarthdataCredentialModeTabs value={mode} onChange={setMode} disabled={loadState !== "ready" || saving.size > 0 || checking} />}
      {passwordField && !(account.id === "earthdata" && mode === "token") ? <ProviderAccountFields
        provider={account.id === "earthdata" ? "Earthdata" : account.title} username={values[userField] ?? ""} password={values[passwordField] ?? ""}
        onUsernameChange={value => edit(userField,value)} onPasswordChange={value => edit(passwordField,value)}
        passwordLabel={account.id === "usgs" ? "Application Token" : t("settings.accounts.password")}
        disabled={loadState !== "ready"}
        oneTimeCode={account.id === "cdse" ? { value: totp, onChange: setTotp } : undefined} />
        : <ProviderCredentialField label={account.id === "earthdata" ? "Earthdata Token" : account.id === "gacos" ? t("settings.accounts.email") : "API Key"}
          credentialLabel={account.id === "gacos" ? t("settings.accounts.gacosEmail") : account.id === "opentopography" ? "OpenTopography Key" : "Earthdata Token"}
          value={values[account.id === "earthdata" ? "earthdata_token" : userField] ?? ""}
          onChange={value => edit(account.id === "earthdata" ? "earthdata_token" : userField, value)}
          secret={account.id !== "gacos"} disabled={loadState !== "ready"} />}
      {external && <p className="text-xs text-muted-foreground">{t("settings.accounts.source", { source: source === "netrc" ? ".netrc" : t("settings.accounts.environment") })}</p>}
      <div className="flex flex-wrap gap-2">
        {account.register && <Button size="sm" variant="outline" type="button" onClick={() => void open(account.register!)}><ExternalLink className="h-3.5 w-3.5" />{t("settings.accounts.register")}</Button>}
        {account.token && (account.id !== "earthdata" || mode === "token") && <Button size="sm" variant="outline" type="button" onClick={() => void open(account.token!)}><ExternalLink className="h-3.5 w-3.5" />{account.id === "opentopography" ? t("settings.accounts.getKey") : t("settings.accounts.getToken")}</Button>}
        {canCheck && <Button size="sm" variant="outline" type="button" onClick={() => void check()}
          disabled={loadState !== "ready" || saving.size > 0 || checking || errorMessages.length > 0 || priorSaveFailures.length > 0 || (!complete && !external)}>
          {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <KeyRound className="h-3.5 w-3.5" />}{t("settings.accounts.checkLogin")}</Button>}
        {account.id === "gacos" && <Button size="sm" variant="outline" type="button" onClick={() => void open("http://www.gacos.net/")}><ExternalLink className="h-3.5 w-3.5" />{t("settings.accounts.website")}</Button>}
        {loadState === "failed" && <Button size="sm" variant="outline" type="button" onClick={() => setLoadAttempt(value => value+1)}>{t("settings.accounts.retryRead")}</Button>}
        {errorMessages.length > 0 && <Button size="sm" variant="outline" type="button" disabled={saving.size > 0} onClick={() => {
          for (const field of account.fields) if (errors[field]) edit(field,values[field] ?? "");
        }}>{t("settings.accounts.retrySave")}</Button>}
      </div>
      {loadState === "failed" && <p role="alert" className="text-xs text-destructive">{t("settings.accounts.readFailed")}</p>}
      {priorSaveFailures.length > 0 && <p role="alert" className="break-words text-xs text-destructive">{t("settings.accounts.previousSaveFailed")}</p>}
      {errorMessages.length > 0 && <p role="alert" className="break-words text-xs text-destructive">{[...new Set(errorMessages)].join(" ")}</p>}
      {feedback && <p role="alert" className="break-words text-xs text-destructive">{feedback}</p>}
    </div>
  </details>;
}
