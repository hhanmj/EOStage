import { getDesktopShellBridge, type DesktopCredentialBridge } from "./desktopShellBridge.ts";
import type { AccountProvider } from "./credentialEditor.ts";
import { loginCdseSession, loginUsgsSession, formatBridgeError } from "./bridge.ts";

export type AccountSessionObservation = { state: "valid" | "unknown" | "invalid" | "unavailable"; message?: string; expiresIn?: number | null };
export async function readAccountSession(provider: AccountProvider, credentials: DesktopCredentialBridge): Promise<AccountSessionObservation> {
  if (provider === "cdse") {
    const result = await getDesktopShellBridge().application.cdseSessionStatus();
    return result.ok ? { state: result.status.authenticated ? "valid" : "unknown", expiresIn: result.status.expires_in_seconds } : { state: "unavailable" };
  }
  if (provider === "usgs") {
    const result = await getDesktopShellBridge().application.usgsSessionStatus();
    return result.ok ? { state: result.status.authenticated ? "valid" : result.status.state === "login_failed" ? "invalid" : "unknown" } : { state: "unavailable" };
  }
  if (provider === "bhoonidhi") {
    const result = await credentials.getBhoonidhiSessionStatus();
    return result.ok ? { state: result.status.authenticated ? "valid" : "unknown", expiresIn: result.status.expires_in_seconds } : { state: "unavailable" };
  }
  // A local presence check cannot establish remote Earthdata/API-key validity.
  return { state: "unknown" };
}

export async function checkAccountLogin(provider: AccountProvider, credentials: DesktopCredentialBridge, totp: string): Promise<AccountSessionObservation> {
  if (provider === "cdse") {
    const result = await loginCdseSession({ username: "", password: "", ...(totp.trim() ? { totp: totp.trim() } : {}) });
    return result.ok ? { state: result.status.authenticated ? "valid" : "invalid", expiresIn: result.status.expires_in_seconds }
      : { state: "invalid", message: formatBridgeError(result) };
  }
  if (provider === "usgs") {
    const result = await loginUsgsSession({ username: "", application_token: "" });
    return result.ok ? { state: result.status.authenticated ? "valid" : "invalid" }
      : { state: "invalid", message: formatBridgeError(result) };
  }
  if (provider === "bhoonidhi") {
    const result = await credentials.checkBhoonidhiAuth();
    return result.ok ? { state: "valid" } : { state: "invalid", message: formatBridgeError(result) };
  }
  if (provider === "earthdata") {
    const result = await credentials.checkEarthdataAuth(true);
    if (!result.ok) return { state: "unavailable", message: formatBridgeError(result) };
    return { state: result.status === "valid" ? "valid" : result.status === "unknown" ? "unknown"
      : result.status === "unavailable" ? "unavailable" : "invalid", message: result.status === "valid" ? undefined : result.message };
  }
  return { state: "unknown" };
}
