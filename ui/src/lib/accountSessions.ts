export type AccountProvider = "cdse" | "usgs_m2m" | "earthdata" | "bhoonidhi" | "mpc" | "opentopography" | "gacos";
export type AccountState = "logged_in" | "login_required" | "credentials_missing" | "service_unavailable" | "not_required";
export type AccountSessionStatus = { provider: AccountProvider; state: AccountState };

export async function loadAccountSessionStatuses(
  invoke: (command: "application_account_session_statuses", payload?: undefined) => Promise<unknown>,
): Promise<AccountSessionStatus[]> {
  const value = await invoke("application_account_session_statuses");
  return parseAccountSessionStatuses(value);
}

const PROVIDERS: readonly AccountProvider[] = ["cdse", "usgs_m2m", "earthdata", "bhoonidhi", "mpc", "opentopography", "gacos"];
const STATES: readonly AccountState[] = ["logged_in", "login_required", "credentials_missing", "service_unavailable", "not_required"];

/** Parse a secret-free desktop projection; never include credential-status strings. */
export function parseAccountSessionStatuses(value: unknown): AccountSessionStatus[] {
  if (!Array.isArray(value) || value.length !== PROVIDERS.length) throw new Error("ACCOUNT_STATUS_INVALID");
  const records = value.map((entry) => {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) throw new Error("ACCOUNT_STATUS_INVALID");
    const record = entry as Record<string, unknown>;
    if (Object.keys(record).length !== 2 || !PROVIDERS.includes(record.provider as AccountProvider) || !STATES.includes(record.state as AccountState)) {
      throw new Error("ACCOUNT_STATUS_INVALID");
    }
    return { provider: record.provider as AccountProvider, state: record.state as AccountState };
  });
  if (new Set(records.map((record) => record.provider)).size !== PROVIDERS.length) throw new Error("ACCOUNT_STATUS_INVALID");
  return records;
}
