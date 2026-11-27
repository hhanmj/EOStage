/**
 * Optimistic persistence helper for UI-level saved-channel and per-model enable switches.
 * Preserves existing AgentChannelSave backend payload schemas while letting saved
 * switches apply immediately with quiet saving state, full rollback on failure, and retry.
 */

import { loadDisabledSkillIds, DISABLED_SKILLS_STORAGE_KEY } from "./agentSkills.ts";
import {
  loadAgentDisabledItemsBackend,
  saveAgentDisabledItemsBackend,
} from "./bridge.ts";

const DISABLED_PROVIDERS_KEY = "eostage.channels.disabledProviders.v1";
const DISABLED_MODELS_KEY = "eostage.channels.disabledModels.v1";

type FailureInjector = ((scope: "provider" | "model", id: string) => Error | null) | null;

let testFailureInjector: FailureInjector = null;

export function setChannelPrefFailureInjectorForTest(fn: FailureInjector): void {
  testFailureInjector = fn;
}

function safeReadMap(key: string): Record<string, boolean> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, boolean>)
      : {};
  } catch {
    return {};
  }
}

export function loadDisabledProviders(): Record<string, boolean> {
  return safeReadMap(DISABLED_PROVIDERS_KEY);
}

export function loadDisabledChannelModels(channelId: string): Record<string, boolean> {
  if (!channelId) return {};
  const all = safeReadMap(DISABLED_MODELS_KEY) as unknown as Record<string, Record<string, boolean>>;
  const entry = all[channelId];
  return entry && typeof entry === "object" ? entry : {};
}

export function loadAllDisabledModelKeys(): string[] {
  const all = safeReadMap(DISABLED_MODELS_KEY) as unknown as Record<string, Record<string, boolean>>;
  const keys: string[] = [];
  for (const [channelId, modelsMap] of Object.entries(all)) {
    if (!modelsMap || typeof modelsMap !== "object") continue;
    for (const [modelId, isDisabled] of Object.entries(modelsMap)) {
      if (isDisabled) keys.push(`${channelId}::${modelId}`);
    }
  }
  return keys;
}

export async function syncDisabledPreferencesToBackend(
  overrideSkills?: string[]
): Promise<void> {
  const disabledProviders = Object.entries(loadDisabledProviders())
    .filter(([, v]) => Boolean(v))
    .map(([k]) => k);
  const disabledModels = loadAllDisabledModelKeys();
  const disabledSkills = overrideSkills ?? Array.from(loadDisabledSkillIds());
  await saveAgentDisabledItemsBackend({
    disabled_providers: disabledProviders,
    disabled_models: disabledModels,
    disabled_skills: disabledSkills,
  }).catch(() => {});
}

export const CHANNEL_PREFERENCES_CHANGED_EVENT = "eostage:channel-preferences-changed";

function notifyPreferencesChanged(): void {
  if (typeof window === "undefined" || typeof window.dispatchEvent !== "function") {
    return;
  }
  try {
    const CustomEventCtor =
      (window as unknown as { CustomEvent?: typeof CustomEvent }).CustomEvent ??
      (typeof CustomEvent === "function" ? CustomEvent : null);
    if (CustomEventCtor) {
      window.dispatchEvent(new CustomEventCtor(CHANNEL_PREFERENCES_CHANGED_EVENT));
    }
  } catch {
    // Ignore cross-realm event dispatch failures in test environments
  }
}

export async function hydrateDisabledPreferencesFromBackend(): Promise<void> {
  const remote = await loadAgentDisabledItemsBackend().catch(() => null);
  if (!remote || typeof window === "undefined") return;
  try {
    const provMap: Record<string, boolean> = {};
    for (const id of remote.disabled_providers ?? []) {
      if (id) provMap[id] = true;
    }
    window.localStorage.setItem(DISABLED_PROVIDERS_KEY, JSON.stringify(provMap));

    const modMap: Record<string, Record<string, boolean>> = {};
    for (const key of remote.disabled_models ?? []) {
      const idx = key.indexOf("::");
      if (idx > 0) {
        const cid = key.slice(0, idx);
        const mid = key.slice(idx + 2);
        if (!modMap[cid]) modMap[cid] = {};
        modMap[cid][mid] = true;
      }
    }
    window.localStorage.setItem(DISABLED_MODELS_KEY, JSON.stringify(modMap));

    if (Array.isArray(remote.disabled_skills)) {
      window.localStorage.setItem(
        DISABLED_SKILLS_STORAGE_KEY,
        JSON.stringify(remote.disabled_skills)
      );
    }
    notifyPreferencesChanged();
  } catch {
    // Storage is optional
  }
}

export async function persistProviderEnabled(channelId: string, enabled: boolean): Promise<void> {
  await Promise.resolve();
  if (testFailureInjector) {
    const injected = testFailureInjector("provider", channelId);
    if (injected) throw injected;
  }
  const map = safeReadMap(DISABLED_PROVIDERS_KEY);
  if (enabled) {
    delete map[channelId];
  } else {
    map[channelId] = true;
  }
  if (typeof window !== "undefined") {
    window.localStorage.setItem(DISABLED_PROVIDERS_KEY, JSON.stringify(map));
  }
  notifyPreferencesChanged();
  void syncDisabledPreferencesToBackend();
}

export async function persistModelEnabled(
  channelId: string,
  modelKey: string,
  enabled: boolean
): Promise<void> {
  await Promise.resolve();
  if (testFailureInjector) {
    const injected = testFailureInjector("model", `${channelId}:${modelKey}`);
    if (injected) throw injected;
  }
  if (typeof window === "undefined") return;
  let all: Record<string, Record<string, boolean>> = {};
  try {
    const raw = window.localStorage.getItem(DISABLED_MODELS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        all = parsed as Record<string, Record<string, boolean>>;
      }
    }
  } catch {
    all = {};
  }
  const nextChannel = { ...(all[channelId] ?? {}) };
  if (enabled) {
    delete nextChannel[modelKey];
  } else {
    nextChannel[modelKey] = true;
  }
  if (Object.keys(nextChannel).length === 0) {
    delete all[channelId];
  } else {
    all[channelId] = nextChannel;
  }
  window.localStorage.setItem(DISABLED_MODELS_KEY, JSON.stringify(all));
  notifyPreferencesChanged();
  void syncDisabledPreferencesToBackend();
}
