import { useI18n } from "@/i18n";
import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import { CheckCircle2, Eye, EyeOff, Loader2, LogIn, Trash2 } from "lucide-react";
import { AccountPortalLinkButton } from "./AccountPortalLinkButton";
import {
  checkBhoonidhiAuth,
  checkEarthdataAuth,
  clearBhoonidhiCredentials,
  clearEarthdataCredentials,
  getCredentialStatus,
  saveBhoonidhiLogin,
  saveEarthdataLogin,
  saveEarthdataToken,
  type CredentialStatus,
} from "@/lib/bridge";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import {
  clearCdseCredentialStore,
  clearUsgsCredentialStore,
  hasSavedProviderLogin,
  loginCdseCredentialSession,
  loginUsgsCredentialSession,
  saveCdseCredentialDraft,
  saveUsgsCredentialDraft,
} from "@/lib/providerCredentialSessions";
import { useAutoSaveDraft } from "./useAutoSaveDraft";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { getDesktopShellBridge, type DesktopCredentialField } from "@/lib/desktopShellBridge";
import { setAccountVerificationFailed } from "@/lib/dataSourceCatalogLogic";

export const CREDENTIALS_CHANGED_EVENT = "eostage:credentials-changed";

type Verification = { ok: boolean; message: string } | null;
type PanelProps = { onChanged?: () => void };

function useCredentialStatus(onChanged?: () => void) {
  const [status, setStatus] = useState<CredentialStatus | null>(null);
  const refresh = useCallback(async () => {
    const next = await getCredentialStatus();
    if (next.ok) setStatus(next);
    onChanged?.();
    window.dispatchEvent(new window.Event(CREDENTIALS_CHANGED_EVENT));
  }, [onChanged]);
  useEffect(() => {
    void getCredentialStatus().then((next) => { if (next.ok) setStatus(next); });
  }, []);
  return { status, refresh };
}

export function CredentialInputField({
  provider,
  label,
  value,
  onChange,
  onBlur,
  secret = false,
  disabled = false,
  autoComplete = "off",
  saved = false,
  revealField,
  labelAccessory,
}: {
  provider: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  onBlur: () => void;
  secret?: boolean;
  disabled?: boolean;
  autoComplete?: string;
  saved?: boolean;
  revealField?: DesktopCredentialField;
  labelAccessory?: ReactNode;
}) {
  const { t } = useI18n();
  const id = useId();
  const [revealed, setRevealed] = useState<string | null>(null);
  const [draftRevealed, setDraftRevealed] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const showRevealSpinner = useDelayedFlag(revealing);
  const [revealError, setRevealError] = useState<string | null>(null);

  const handleChange = (nextVal: string) => {
    if (revealed !== null) {
      setRevealed(null);
    }
    if (!nextVal) {
      setDraftRevealed(false);
    }
    onChange(nextVal);
  };

  const toggleReveal = async () => {
    if (secret && value) {
      setDraftRevealed((prev) => !prev);
      setRevealError(null);
      return;
    }
    if (revealed !== null) {
      setRevealed(null);
      setRevealError(null);
      return;
    }
    if (!revealField) return;
    setRevealing(true);
    setRevealError(null);
    try {
      const credentials = getDesktopShellBridge().credentials;
      if (!credentials) return;
      const res = await credentials.reveal(revealField);
      if (res?.ok && res.value !== null) {
        setRevealed(res.value);
      } else {
        setRevealError(t("accounts.fields.readFailed"));
      }
    } catch {
      setRevealError(t("accounts.fields.readFailed"));
    } finally {
      setRevealing(false);
    }
  };

  const isShowingMask = saved && secret && !value && revealed === null;
  const isRevealed = revealed !== null;
  const isSecretVisible = isRevealed || (secret && Boolean(value) && draftRevealed);
  const displayValue = isRevealed ? revealed : value;
  const inputType = secret && !isSecretVisible ? "password" : "text";
  const canToggleVisibility = secret && ((Boolean(revealField) && saved) || Boolean(value));

  return (
    <div>
      <div className="mb-1 flex items-center justify-between gap-2 text-[13px]">
        <div className="flex items-center gap-1.5 min-w-0 flex-wrap">
          <label htmlFor={id} className="inline-flex items-center flex-wrap gap-2 text-[13px] text-sp-text-secondary">
            <span className="whitespace-nowrap">{label}</span>
            {saved && (
              <span className="inline-flex items-center rounded-full bg-sp-success-bg px-2 py-1 text-xs font-normal leading-none text-sp-success-text whitespace-nowrap shrink-0">
                {t("accounts.fields.saved")}
              </span>
            )}
          </label>
          {labelAccessory}
        </div>
      </div>
      <div className="relative">
        <input
          id={id}
          aria-label={t("accounts.fields.credentialLabel", { provider, label })}
          type={inputType}
          value={displayValue}
          placeholder={isShowingMask ? "••••••••" : ""}
          onChange={(event) => handleChange(event.currentTarget.value)}
          onBlur={onBlur}
          disabled={disabled || revealing}
          readOnly={isRevealed}
          autoComplete={autoComplete}
          spellCheck={false}
          className={`eo-settings-input w-full min-w-0 text-[13px] ${
            secret ? "font-mono placeholder:font-sans" : "font-sans"
          } disabled:opacity-50 ${
            canToggleVisibility ? "pr-9" : ""
          }`}
        />
        {canToggleVisibility && (
          <button
            type="button"
            aria-label={isSecretVisible ? t("accounts.fields.hide") : t("accounts.fields.show")}
            title={isSecretVisible ? t("accounts.fields.hide") : t("accounts.fields.show")}
            disabled={disabled || revealing}
            onClick={() => void toggleReveal()}
            className="absolute right-1 top-1/2 -translate-y-1/2 rounded-sp-sm p-1 text-[13px] text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text focus:outline-none disabled:opacity-40"
          >
            {showRevealSpinner ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : isSecretVisible ? (
              <EyeOff className="h-4 w-4 text-sp-accent-strong" />
            ) : (
              <Eye className="h-4 w-4" />
            )}
          </button>
        )}
      </div>
      {revealError && <p className="mt-1 text-xs text-sp-danger-text">{revealError}</p>}
    </div>
  );
}

function PanelActions({
  provider,
  verifyLabel,
  verifying,
  clearing,
  verifyDisabled,
  clearDisabled,
  onVerify,
  onClear,
}: {
  provider: string;
  verifyLabel: string;
  verifying: boolean;
  clearing: boolean;
  verifyDisabled: boolean;
  clearDisabled: boolean;
  onVerify: () => void;
  onClear: () => void;
}) {
  const { t } = useI18n();
  const showVerifySpinner = useDelayedFlag(verifying);
  const showClearSpinner = useDelayedFlag(clearing);
  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={onVerify}
        aria-busy={verifying}
        disabled={verifyDisabled}
        className="cred-action-btn inline-flex h-8 min-w-[84px] items-center justify-center gap-2 px-3 text-[13px]"
      >
        {showVerifySpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : <LogIn className="h-3.5 w-3.5 shrink-0" />}
        <span>{verifyLabel}</span>
      </button>
      <button
        type="button"
        aria-label={t("accounts.fields.clearProvider", { provider })}
        aria-busy={clearing}
        onClick={onClear}
        disabled={clearDisabled}
        className="inline-flex h-8 min-w-[76px] items-center justify-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-[13px] font-normal text-sp-danger-text transition-colors hover:bg-sp-danger-bg disabled:cursor-not-allowed disabled:opacity-40 disabled:text-sp-text-muted disabled:hover:bg-sp-surface"
      >
        {showClearSpinner ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" /> : <Trash2 className="h-3.5 w-3.5 shrink-0" />}
        <span>{t("accounts.clear")}</span>
      </button>
    </div>
  );
}

function PanelFeedback({
  saving,
  saved,
  saveError,
  verification,
}: {
  saving: boolean;
  saved: boolean;
  saveError: string | null;
  verification: Verification;
}) {
  const { t } = useI18n();
  return (
    <>
      {saving && <p role="status" className="text-xs text-sp-text-muted">{t("accounts.saving")}</p>}
      {saved && !verification && <p role="status" className="text-xs text-sp-text-muted">{t("accounts.saved")}</p>}
      {saveError && <p role="alert" className="break-words text-xs text-sp-danger-text">{saveError}</p>}
      {verification && (
        <p
          role={verification.ok ? "status" : "alert"}
          className={`inline-flex items-start gap-1.5 break-words text-xs ${verification.ok ? "text-sp-success-text" : "text-sp-danger-text"}`}
        >
          {verification.ok && <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
          {verification.message}
        </p>
      )}
    </>
  );
}

export function DesktopCdseAccountPanel({ onChanged }: PanelProps) {
  const { t } = useI18n();
  const { status, refresh } = useCredentialStatus(onChanged);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [totp, setTotp] = useState("");
  const [verification, setVerification] = useState<Verification>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"verify" | "clear" | null>(null);
  const configured = hasSavedProviderLogin(status?.cdse);

  // 回显已保存的用户名（非密文字段）
  useEffect(() => {
    let cancelled = false;
    if (configured && !username) {
      void getDesktopShellBridge().credentials?.readForEdit("cdse_username").then((res) => {
        if (!cancelled && res?.ok && res.value) {
          setUsername(res.value);
        }
      });
    }
    return () => { cancelled = true; };
  }, [configured]);

  const complete = Boolean(username.trim() && password);
  const autoSave = useAutoSaveDraft({
    draftKey: `${username}\0${password}`,
    complete,
    save: () => saveCdseCredentialDraft(username.trim(), password),
    onSaved: async () => { setVerification(null); await refresh(); },
  });
  const editing = Boolean(password);
  const locked = busy !== null;

  async function verify() {
    setBusy("verify");
    setVerification(null);
    setActionError(null);
    try {
      const result = await loginCdseCredentialSession({
        savedStatus: status?.cdse,
        usernameDraft: "",
        password: "",
        totpDraft: totp,
      });
      setVerification({ ok: result.ok, message: result.ok ? t("accounts.valid") : result.error });
      await refresh();
    } catch (error) {
      setVerification({ ok: false, message: formatBridgeErrorValue(error) });
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    setBusy("clear");
    setActionError(null);
    try {
      const result = await clearCdseCredentialStore(false);
      if (!result.ok) {
        setActionError(result.error);
        return;
      }
      setUsername("");
      setPassword("");
      setTotp("");
      setVerification(null);
      autoSave.reset();
      await refresh();
    } catch (error) {
      setActionError(formatBridgeErrorValue(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-3" data-provider-session-panel="cdse">
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <CredentialInputField provider="CDSE" label={t("accounts.username")} value={username} onChange={(value) => { setUsername(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} autoComplete="username" saved={configured} />
        <CredentialInputField provider="CDSE" label={t("accounts.password")} value={password} onChange={(value) => { setPassword(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret autoComplete="current-password" saved={configured} revealField="cdse_password" />
      </div>
      <CredentialInputField provider="CDSE" label={t("accounts.otp")} value={totp} onChange={(value) => { setTotp(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret autoComplete="one-time-code" />
      <PanelActions provider="CDSE" verifyLabel={t("accounts.login")} verifying={busy === "verify"} clearing={busy === "clear"} onVerify={() => void verify()} onClear={() => void clear()}
        verifyDisabled={locked || autoSave.state.status === "saving" || !configured || (editing && (!complete || autoSave.state.status !== "saved"))}
        clearDisabled={locked || autoSave.state.status === "saving" || !configured} />
      <PanelFeedback saving={autoSave.state.status === "saving"} saved={autoSave.state.status === "saved"} saveError={actionError ?? autoSave.state.error} verification={verification} />
    </div>
  );
}

export function DesktopUsgsAccountPanel({ onChanged }: PanelProps) {
  const { t } = useI18n();
  const { status, refresh } = useCredentialStatus(onChanged);
  const [username, setUsername] = useState("");
  const [token, setToken] = useState("");
  const [verification, setVerification] = useState<Verification>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"verify" | "clear" | null>(null);
  const configured = hasSavedProviderLogin(status?.usgs);

  // 回显已保存的用户名（非密文字段）
  useEffect(() => {
    let cancelled = false;
    if (configured && !username) {
      void getDesktopShellBridge().credentials?.readForEdit("usgs_username").then((res) => {
        if (!cancelled && res?.ok && res.value) {
          setUsername(res.value);
        }
      });
    }
    return () => { cancelled = true; };
  }, [configured]);

  const complete = Boolean(username.trim() && token);
  const autoSave = useAutoSaveDraft({
    draftKey: `${username}\0${token}`,
    complete,
    save: () => saveUsgsCredentialDraft(username.trim(), token),
    onSaved: async () => { setVerification(null); await refresh(); },
  });
  const editing = Boolean(token);
  const locked = busy !== null;

  async function verify() {
    setBusy("verify");
    setVerification(null);
    setActionError(null);
    try {
      const result = await loginUsgsCredentialSession({
        savedStatus: status?.usgs,
        usernameDraft: token ? username : "",
        applicationToken: token || "",
      });
      setVerification({ ok: result.ok, message: result.ok ? t("accounts.valid") : result.error });
      await refresh();
    } catch (error) {
      setVerification({ ok: false, message: formatBridgeErrorValue(error) });
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    setBusy("clear");
    setActionError(null);
    try {
      const result = await clearUsgsCredentialStore(false);
      if (!result.ok) {
        setActionError(result.error);
        return;
      }
      setUsername("");
      setToken("");
      setVerification(null);
      autoSave.reset();
      await refresh();
    } catch (error) {
      setActionError(formatBridgeErrorValue(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-3" data-provider-session-panel="usgs">
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <CredentialInputField provider="USGS" label={t("accounts.username")} value={username} onChange={(value) => { setUsername(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} autoComplete="username" saved={configured} />
        <CredentialInputField provider="USGS" label="Application Token" value={token} onChange={(value) => { setToken(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret saved={configured} revealField="usgs_application_token" labelAccessory={<AccountPortalLinkButton target="usgs_token" onError={setActionError} />} />
      </div>
      <PanelActions provider="USGS" verifyLabel={t("accounts.login")} verifying={busy === "verify"} clearing={busy === "clear"} onVerify={() => void verify()} onClear={() => void clear()}
        verifyDisabled={locked || autoSave.state.status === "saving" || !configured || (editing && (!complete || autoSave.state.status !== "saved"))}
        clearDisabled={locked || autoSave.state.status === "saving" || !configured} />
      <PanelFeedback saving={autoSave.state.status === "saving"} saved={autoSave.state.status === "saved"} saveError={actionError ?? autoSave.state.error} verification={verification} />
    </div>
  );
}

export function DesktopBhoonidhiAccountPanel({ onChanged }: PanelProps) {
  const { t } = useI18n();
  const { status, refresh } = useCredentialStatus(onChanged);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [verification, setVerification] = useState<Verification>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"verify" | "clear" | null>(null);
  const configured = hasSavedProviderLogin(status?.bhoonidhi);

  // 回显已保存的用户名（非密文字段）
  useEffect(() => {
    let cancelled = false;
    if (configured && !username) {
      void getDesktopShellBridge().credentials?.readForEdit("bhoonidhi_username").then((res) => {
        if (!cancelled && res?.ok && res.value) {
          setUsername(res.value);
        }
      });
    }
    return () => { cancelled = true; };
  }, [configured]);

  const complete = Boolean(username.trim() && password);
  const autoSave = useAutoSaveDraft({
    draftKey: `${username}\0${password}`,
    complete,
    save: () => saveBhoonidhiLogin(username.trim(), password),
    onSaved: async () => { setVerification(null); setAccountVerificationFailed("bhoonidhi", false); await refresh(); },
  });
  const editing = Boolean(password);
  const locked = busy !== null;

  async function verify() {
    setBusy("verify");
    setVerification(null);
    setActionError(null);
    try {
      const result = await checkBhoonidhiAuth();
      setAccountVerificationFailed("bhoonidhi", !result.ok);
      setVerification({ ok: result.ok, message: result.ok ? t("accounts.valid") : formatBridgeErrorValue(result) });
      await refresh();
    } catch (error) {
      setVerification({ ok: false, message: formatBridgeErrorValue(error) });
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    setBusy("clear");
    setActionError(null);
    try {
      const result = await clearBhoonidhiCredentials();
      if (!result.ok) {
        setActionError(formatBridgeErrorValue(result));
        return;
      }
      setUsername("");
      setPassword("");
      setVerification(null);
      autoSave.reset();
      await refresh();
    } catch (error) {
      setActionError(formatBridgeErrorValue(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-3" data-provider-session-panel="bhoonidhi">
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <CredentialInputField provider="Bhoonidhi" label={t("accounts.username")} value={username} onChange={(value) => { setUsername(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} autoComplete="username" saved={configured} />
        <CredentialInputField provider="Bhoonidhi" label={t("accounts.password")} value={password} onChange={(value) => { setPassword(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret autoComplete="current-password" saved={configured} revealField="bhoonidhi_password" />
      </div>
      <PanelActions provider="Bhoonidhi" verifyLabel={t("accounts.verify")} verifying={busy === "verify"} clearing={busy === "clear"} onVerify={() => void verify()} onClear={() => void clear()}
        verifyDisabled={locked || autoSave.state.status === "saving" || !configured || (editing && (!complete || autoSave.state.status !== "saved"))}
        clearDisabled={locked || autoSave.state.status === "saving" || !configured} />
      <PanelFeedback saving={autoSave.state.status === "saving"} saved={autoSave.state.status === "saved"} saveError={actionError ?? autoSave.state.error} verification={verification} />
    </div>
  );
}

export function DesktopEarthdataAccountPanel({ onChanged }: PanelProps) {
  const { t } = useI18n();
  const { status, refresh } = useCredentialStatus(onChanged);
  const [mode, setMode] = useState<"token" | "login">("token");
  const [token, setToken] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [verification, setVerification] = useState<Verification>(null);
  const [netrcValidity, setNetrcValidity] = useState<"valid" | "invalid" | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"verify" | "clear" | null>(null);
  const configured = Boolean(status?.earthdata && !["none", "unavailable"].includes(status.earthdata));

  // 回显已保存的用户名（非密文字段）
  useEffect(() => {
    let cancelled = false;
    if (configured && !username && mode === "login") {
      void getDesktopShellBridge().credentials?.readForEdit("earthdata_username").then((res) => {
        if (!cancelled && res?.ok && res.value) {
          setUsername(res.value);
        }
      });
    }
    return () => { cancelled = true; };
  }, [configured, mode]);

  const hasSavedToken = configured && status?.earthdata === "token";
  const hasSavedLogin = configured && Boolean(status?.earthdata?.startsWith("login:"));
  const complete = mode === "token" ? Boolean(token.trim()) : Boolean(username.trim() && password);
  const autoSave = useAutoSaveDraft({
    draftKey: `${mode}\0${token}\0${username}\0${password}`,
    complete,
    save: () => mode === "token" ? saveEarthdataToken(token.trim()) : saveEarthdataLogin(username.trim(), password),
    onSaved: async () => { setVerification(null); setNetrcValidity(null); await refresh(); },
  });
  const editing = mode === "token" ? Boolean(token) : Boolean(password);
  const locked = busy !== null;

  useEffect(() => {
    const savedMode =
      status?.earthdata_input?.mode ??
      (status?.earthdata?.startsWith("login:")
        ? "login"
        : status?.earthdata === "token"
          ? "token"
          : undefined);
    if ((savedMode === "token" || savedMode === "login") && !token && !username && !password) setMode(savedMode);
  }, [status?.earthdata, status?.earthdata_input?.mode, token, username, password]);

  async function verify() {
    setBusy("verify");
    setVerification(null);
    setActionError(null);
    try {
      const result = await checkEarthdataAuth(true);
      const valid = result.ok && result.status === "valid";
      const definitelyInvalid =
        result.ok && (result.status === "invalid" || result.status === "expired");
      setAccountVerificationFailed("earthdata", definitelyInvalid);
      const message = valid
        ? t("accounts.valid")
        : result.ok
          ? result.message || t("accounts.invalid")
          : formatBridgeErrorValue(result);
      setVerification({ ok: valid, message });
      if (status?.earthdata === "netrc") {
        setNetrcValidity(valid ? "valid" : definitelyInvalid ? "invalid" : null);
      }
      await refresh();
    } catch (error) {
      setVerification({ ok: false, message: formatBridgeErrorValue(error) });
      if (status?.earthdata === "netrc") setNetrcValidity("invalid");
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    setBusy("clear");
    setActionError(null);
    try {
      const result = await clearEarthdataCredentials();
      if (!result.ok) {
        setActionError(formatBridgeErrorValue(result));
        return;
      }
      setToken("");
      setUsername("");
      setPassword("");
      setVerification(null);
      setNetrcValidity(null);
      autoSave.reset();
      await refresh();
    } catch (error) {
      setActionError(formatBridgeErrorValue(error));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-3" data-provider-session-panel="earthdata">
      {status?.earthdata === "netrc" && (
        <p className={`text-xs ${netrcValidity === "valid" ? "text-sp-success-text" : netrcValidity === "invalid" ? "text-sp-danger-text" : "text-sp-text-secondary"}`}>
          {t("accounts.earthdata.netrcDetected")}{netrcValidity ? ` · ${t(netrcValidity === "valid" ? "accounts.status.valid" : "accounts.status.invalid")}` : ""}
        </p>
      )}
      <div role="group" aria-label={t("accounts.earthdata.mode")} className="inline-flex rounded-sp-sm bg-sp-surface-muted p-1">
        {(["token", "login"] as const).map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={mode === value}
            disabled={locked}
            onClick={() => { setMode(value); setVerification(null); setNetrcValidity(null); }}
            className={`rounded-sp-sm px-3 py-1.5 text-[13px] ${mode === value ? "bg-sp-surface text-sp-text shadow-card" : "text-sp-text-secondary hover:text-sp-text"}`}
          >
            {value === "token" ? "Token" : t("accounts.earthdata.loginMode")}
          </button>
        ))}
      </div>
      {mode === "token" ? (
        <CredentialInputField provider="Earthdata" label="Token" value={token} onChange={(value) => { setToken(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret saved={hasSavedToken} revealField="earthdata_token" labelAccessory={<AccountPortalLinkButton target="earthdata_token" onError={setActionError} />} />
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          <CredentialInputField provider="Earthdata" label={t("accounts.username")} value={username} onChange={(value) => { setUsername(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} autoComplete="username" saved={hasSavedLogin} />
          <CredentialInputField provider="Earthdata" label={t("accounts.password")} value={password} onChange={(value) => { setPassword(value); setVerification(null); }} onBlur={() => void autoSave.saveNow()} disabled={locked} secret autoComplete="current-password" saved={hasSavedLogin} revealField="earthdata_password" />
        </div>
      )}
      <PanelActions provider="Earthdata" verifyLabel={t("accounts.verify")} verifying={busy === "verify"} clearing={busy === "clear"} onVerify={() => void verify()} onClear={() => void clear()}
        verifyDisabled={locked || autoSave.state.status === "saving" || !configured || (editing && (!complete || autoSave.state.status !== "saved"))}
        clearDisabled={locked || autoSave.state.status === "saving" || !configured} />
      <PanelFeedback saving={autoSave.state.status === "saving"} saved={autoSave.state.status === "saved"} saveError={actionError ?? autoSave.state.error} verification={verification} />
    </div>
  );
}
