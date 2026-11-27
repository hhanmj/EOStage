import type { AgentChannelView } from "./agentBridgeTypes.ts";

export type ChannelHealthStatus = "ok" | "error";

export interface ChannelHealthEntry {
  status: ChannelHealthStatus;
  checkedAt: number;
  message?: string;
}

export type ChannelHealthDotState = "ok" | "error" | "disabled" | "missing_key" | "untested";

const CHANNEL_HEALTH_STORAGE_KEY = "eostage.channel.health.v1";

const memoryFallbackMap: Record<string, ChannelHealthEntry> = {};
const listeners = new Set<() => void>();

function notifyListeners() {
  for (const listener of listeners) {
    try {
      listener();
    } catch {
      // Ignore subscriber errors
    }
  }
}

export function getChannelHealthMap(): Record<string, ChannelHealthEntry> {
  try {
    if (typeof window !== "undefined" && typeof window.localStorage !== "undefined") {
      const raw = window.localStorage.getItem(CHANNEL_HEALTH_STORAGE_KEY);
      if (!raw) return {};
      const parsed = JSON.parse(raw) as unknown;
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, ChannelHealthEntry>;
      }
      return {};
    }
  } catch {
    // Fall back to in-memory map
  }
  return { ...memoryFallbackMap };
}

function writeChannelHealthMap(next: Record<string, ChannelHealthEntry>): void {
  Object.keys(memoryFallbackMap).forEach((k) => {
    delete memoryFallbackMap[k];
  });
  Object.assign(memoryFallbackMap, next);
  try {
    if (typeof window !== "undefined" && typeof window.localStorage !== "undefined") {
      window.localStorage.setItem(CHANNEL_HEALTH_STORAGE_KEY, JSON.stringify(next));
    }
  } catch {
    // Ignore storage write errors
  }
  notifyListeners();
}

export function getChannelHealth(channelId: string): ChannelHealthEntry | null {
  const id = channelId.trim();
  if (!id) return null;
  const map = getChannelHealthMap();
  return map[id] ?? null;
}

export function setChannelHealth(
  channelId: string,
  status: ChannelHealthStatus,
  message?: string,
): void {
  const id = channelId.trim();
  if (!id) return;
  const map = getChannelHealthMap();
  map[id] = {
    status,
    checkedAt: Date.now(),
    ...(message && message.trim() ? { message: message.trim() } : {}),
  };
  writeChannelHealthMap(map);
}

export function clearChannelHealth(channelId: string): void {
  const id = channelId.trim();
  if (!id) return;
  const map = getChannelHealthMap();
  if (!(id in map)) return;
  delete map[id];
  writeChannelHealthMap(map);
}

export function subscribeChannelHealth(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function resolveChannelHealthDotState(
  channel: Pick<AgentChannelView, "id" | "key_ready">,
  health?: ChannelHealthEntry | null,
  enabled = true,
): ChannelHealthDotState {
  if (!enabled) return "disabled";
  if (health?.status === "error") return "error";
  if (!channel.key_ready) return "missing_key";
  return "ok";
}
