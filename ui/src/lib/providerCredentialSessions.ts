import { t } from "@/i18n";
import {
  clearCdseCredentials,
  clearUsgsCredentials,
  formatBridgeError,
  getCdseSessionStatus,
  getUsgsSessionStatus,
  loginCdseSession,
  loginUsgsSession,
  logoutCdseSession,
  logoutUsgsSession,
  saveCdseLogin,
  saveUsgsLogin,
} from "@/lib/bridge";

export function hasSavedProviderLogin(status: string | null | undefined) {
  return Boolean(status?.startsWith("login:"));
}

export type CredentialSessionAction<T = undefined> =
  | { ok: true; value: T; note: string }
  | { ok: false; error: string; code?: string };

function extractErrorCode(error: unknown): string | undefined {
  if (!error || typeof error !== "object") return undefined;
  const obj = error as Record<string, unknown>;
  const raw =
    typeof obj.code === "string" && obj.code.trim()
      ? obj.code.trim()
      : typeof obj.error_code === "string" && obj.error_code.trim()
        ? obj.error_code.trim()
        : "";
  return raw || undefined;
}

function failed(error: unknown): CredentialSessionAction<never> {
  const code = extractErrorCode(error);
  return code ? { ok: false, error: formatBridgeError(error), code } : { ok: false, error: formatBridgeError(error) };
}

export async function refreshCdseCredentialSession() {
  try {
    const result = await getCdseSessionStatus();
    return result.ok
      ? ({ ok: true, value: result.status, note: t("accounts.feedback.refreshed") } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

/**
 * Restore the process-local CDSE session only when a user-requested delivery
 * operation needs it. Passive status refresh remains network-free.
 */
export async function ensureCdseCredentialSession(savedStatus: string | null | undefined) {
  const current = await refreshCdseCredentialSession();
  if (!current.ok || current.value.authenticated || !hasSavedProviderLogin(savedStatus)) {
    return current;
  }
  return loginCdseCredentialSession({
    savedStatus,
    usernameDraft: "",
    password: "",
    totpDraft: "",
  });
}

export async function saveCdseCredentialDraft(usernameDraft: string, password: string) {
  const username = usernameDraft.trim();
  if (!username || !password) {
    return { ok: false, error: t("accounts.sessions.cdseDraftRequired") } as const;
  }
  try {
    const result = await saveCdseLogin(username, password);
    return result.ok
      ? ({ ok: true, value: undefined, note: t("accounts.feedback.saved") } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function clearCdseCredentialStore(sessionAuthenticated: boolean) {
  try {
    const result = await clearCdseCredentials();
    return result.ok
      ? ({
          ok: true,
          value: undefined,
          note: sessionAuthenticated ? t("accounts.feedback.clearedSessionActive") : t("accounts.feedback.cleared"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function loginCdseCredentialSession(input: {
  savedStatus: string | null | undefined;
  usernameDraft: string;
  password: string;
  totpDraft: string;
}) {
  const username = input.usernameDraft.trim();
  const password = input.password;
  const totp = input.totpDraft.trim();
  const hasDraft = Boolean(username || password);
  if (hasDraft && (!username || !password)) {
    return {
      ok: false,
      error: t("accounts.sessions.cdseDraftIncomplete"),
    } as const;
  }
  if (!hasDraft && !hasSavedProviderLogin(input.savedStatus)) {
    return { ok: false, error: t("accounts.sessions.cdseLoginRequired") } as const;
  }
  try {
    const result = await loginCdseSession({
      username: hasDraft ? username : "",
      password: hasDraft ? password : "",
      ...(totp ? { totp } : {}),
    });
    return result.ok
      ? ({
          ok: true,
          value: result.status,
          note: t("accounts.feedback.loginAvailable"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function logoutCdseCredentialSession() {
  try {
    const result = await logoutCdseSession();
    return result.ok
      ? ({
          ok: true,
          value: result.status,
          note: t("accounts.feedback.loggedOut"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function refreshUsgsCredentialSession() {
  try {
    const result = await getUsgsSessionStatus();
    return result.ok
      ? ({ ok: true, value: result.status, note: t("accounts.feedback.refreshed") } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

/**
 * Restore the process-local USGS session from the native saved login on demand.
 * This is deliberately not called by passive focus/interval status refresh.
 *
 * A login that is already in flight (startup restore, another admission) is
 * awaited by the backend and its outcome reused, so the login itself cannot
 * fail from the race. What can still race at app start is the first status
 * read: if it errors, re-check once before giving up, so a momentary backend
 * hiccup does not surface as "无法建立下载会话".
 */
export async function ensureUsgsCredentialSession(savedStatus: string | null | undefined) {
  let current = await refreshUsgsCredentialSession();
  if (!current.ok && hasSavedProviderLogin(savedStatus)) {
    current = await refreshUsgsCredentialSession();
  }
  if (!current.ok || current.value.authenticated || !hasSavedProviderLogin(savedStatus)) {
    return current;
  }
  const ensured = await loginUsgsCredentialSession({
    savedStatus,
    usernameDraft: "",
    applicationToken: "",
  });
  // A login may have completed concurrently while ours was in flight; only
  // report failure when the session is still not authenticated.
  if (!ensured.ok || !ensured.value.authenticated) {
    const rechecked = await refreshUsgsCredentialSession();
    if (rechecked.ok && rechecked.value.authenticated) {
      return rechecked;
    }
  }
  return ensured;
}

export async function saveUsgsCredentialDraft(usernameDraft: string, applicationToken: string) {
  const username = usernameDraft.trim();
  if (!username || !applicationToken) {
    return {
      ok: false,
      error: t("accounts.sessions.usgsDraftRequired"),
    } as const;
  }
  try {
    const result = await saveUsgsLogin(username, applicationToken);
    return result.ok
      ? ({
          ok: true,
          value: undefined,
          note: t("accounts.feedback.saved"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function clearUsgsCredentialStore(sessionAuthenticated: boolean) {
  try {
    const result = await clearUsgsCredentials();
    return result.ok
      ? ({
          ok: true,
          value: undefined,
          note: sessionAuthenticated ? t("accounts.feedback.clearedSessionActive") : t("accounts.feedback.cleared"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function loginUsgsCredentialSession(input: {
  savedStatus: string | null | undefined;
  usernameDraft: string;
  applicationToken: string;
}) {
  const username = input.usernameDraft.trim();
  const applicationToken = input.applicationToken;
  const hasDraft = Boolean(username || applicationToken);
  if (hasDraft && (!username || !applicationToken)) {
    return {
      ok: false,
      error: t("accounts.sessions.usgsDraftIncomplete"),
    } as const;
  }
  if (!hasDraft && !hasSavedProviderLogin(input.savedStatus)) {
    return {
      ok: false,
      error: t("accounts.sessions.usgsLoginRequired"),
    } as const;
  }
  try {
    const result = await loginUsgsSession({
      username: hasDraft ? username : "",
      application_token: hasDraft ? applicationToken : "",
    });
    return result.ok
      ? ({
          ok: true,
          value: result.status,
          note: t("accounts.feedback.loginAvailable"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}

export async function logoutUsgsCredentialSession() {
  try {
    const result = await logoutUsgsSession();
    return result.ok
      ? ({
          ok: true,
          value: result.status,
          note: t("accounts.feedback.loggedOut"),
        } as const)
      : failed(result);
  } catch (error) {
    return failed(error);
  }
}
