import { taskDeliveryProvider, type SarTaskNameSource } from "./sarTaskDisplayName.ts";
import type { CredentialSessionAction } from "./providerCredentialSessions.ts";

export type SessionProviderKey = "cdse" | "usgs";

const SESSION_PROVIDERS: Readonly<Record<string, { key: SessionProviderKey; label: string }>> = {
  "copernicus.dataspace.odata": { key: "cdse", label: "CDSE" },
  "usgs.eros.m2m": { key: "usgs", label: "USGS" },
};

export type ProviderSessionGateResult =
  | { ok: true; provider: SessionProviderKey | null }
  | { ok: false; provider: SessionProviderKey; label: string; message: string; reason: string | null };

export type ProviderSessionEnsurers = Record<
  SessionProviderKey,
  () => Promise<CredentialSessionAction<{ authenticated: boolean }>>
>;

/** The process-local login session a task needs before it can resume, if any. */
export function taskSessionProvider(
  task: SarTaskNameSource | null | undefined,
): { key: SessionProviderKey; label: string } | null {
  if (!task) return null;
  return SESSION_PROVIDERS[taskDeliveryProvider(task).id] ?? null;
}

/**
 * Restore the provider session of an official-archive task before it is resumed,
 * retried or restored. Sessions live only in the running app, so after a restart
 * the task would otherwise fail as DL004. Only saved credentials are used.
 */
export async function ensureProviderSessionForTask(
  task: SarTaskNameSource | null | undefined,
  ensure: ProviderSessionEnsurers,
  formatError: (error: unknown) => string,
): Promise<ProviderSessionGateResult> {
  const provider = taskSessionProvider(task);
  if (!provider) return { ok: true, provider: null };
  const blocked = (reason: string | null): ProviderSessionGateResult => ({
    ok: false,
    provider: provider.key,
    label: provider.label,
    message: `需要登录 ${provider.label} 后才能继续`,
    reason,
  });
  try {
    const result = await ensure[provider.key]();
    if (!result.ok) return blocked(result.error);
    return result.value.authenticated === true ? { ok: true, provider: provider.key } : blocked(null);
  } catch (error) {
    return blocked(formatError(error));
  }
}
