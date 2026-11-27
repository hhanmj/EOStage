/**
 * EOStage W5 Shell Navigation Routes and Pure Helpers
 *
 * Provides route parsing, fallback resolution, route metadata,
 * and injectable persistence without React or DOM dependencies.
 */

import { t } from "../../i18n/index.ts";

export type ShellRoute =
  | "agent"
  | "history"
  | "workbench"
  | "downloads"
  | "plugins"
  | "settings";

export const SHELL_PAGE_ROUTES: readonly ShellRoute[] = [
  "agent",
  "history",
  "workbench",
  "downloads",
  "plugins",
] as const;

export const SHELL_LAST_ROUTE_KEY = "eostage.shell.lastRoute";

export interface RouteMeta {
  label: string;
}

export const ROUTE_METADATA: Record<ShellRoute, RouteMeta> = {
  agent: { get label() { return t("shared.shellRoutes.agent"); } },
  history: { get label() { return t("shared.shellRoutes.history"); } },
  workbench: { get label() { return t("shared.shellRoutes.workbench"); } },
  downloads: { get label() { return t("shared.shellRoutes.downloads"); } },
  plugins: { get label() { return t("shared.shellRoutes.plugins"); } },
  settings: { get label() { return t("shared.shellRoutes.settings"); } },
};

/**
 * Parses strict shell routes from a window hash.
 * Only the five exact hash forms (#/agent, #/history, #/workbench, #/downloads, #/settings) are recognized.
 * Query strings, prefixes, or arbitrary hashes return null.
 */
export function parseShellRoute(hash: string): ShellRoute | null {
  switch (hash) {
    case "#/agent":
      return "agent";
    case "#/history":
      return "history";
    case "#/workbench":
      return "workbench";
    case "#/downloads":
      return "downloads";
    case "#/plugins":
      return "plugins";
    case "#/settings":
      return "settings";
    default:
      return null;
  }
}

/**
 * Returns metadata (e.g. human-readable label in Simplified Chinese) for a shell route.
 */
export function routeMeta(route: ShellRoute): RouteMeta {
  return ROUTE_METADATA[route];
}

/**
 * Resolves the initial route:
 * 1. If currentHash parses to a valid route, use it.
 * 2. Else if stored is one of the four persistent page routes (agent/history/workbench/downloads), use it.
 * 3. Otherwise fall back to "agent".
 */
export function resolveInitialRoute(
  currentHash: string,
  stored?: string | null
): ShellRoute {
  const parsed = parseShellRoute(currentHash);
  if (parsed) {
    return parsed;
  }

  if (
    stored === "agent" ||
    stored === "history" ||
    stored === "workbench" ||
    stored === "downloads" ||
    stored === "plugins"
  ) {
    return stored;
  }

  return "agent";
}

/**
 * Computes the hash to write on initial mount so window.location.hash matches the resolved route.
 * Returns null if currentHash already parses to a valid shell route or contains dev switches ("settings" / "legacy").
 */
export function initialHashToWrite(
  currentHash: string,
  route: ShellRoute
): string | null {
  if (parseShellRoute(currentHash) !== null) {
    return null;
  }
  if (currentHash.includes("settings") || currentHash.includes("legacy")) {
    return null;
  }
  return `#/${route}`;
}

export interface SimpleStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/**
 * Reads the last persisted shell page route from the given storage.
 * Returns null if stored route is missing, invalid, or "settings".
 */
export function readLastRoute(storage?: SimpleStorage | null): ShellRoute | null {
  if (!storage) return null;
  try {
    const val = storage.getItem(SHELL_LAST_ROUTE_KEY);
    if (
      val === "agent" ||
      val === "history" ||
      val === "workbench" ||
      val === "downloads" ||
      val === "plugins"
    ) {
      return val;
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * Persists the current shell route.
 * Writing "settings" or invalid route values is strictly a no-op (settings is not persisted).
 */
export function writeLastRoute(
  storage: SimpleStorage | null | undefined,
  route: ShellRoute
): void {
  if (!storage) return;
  if (route === "settings") return;
  if (
    route === "agent" ||
    route === "history" ||
    route === "workbench" ||
    route === "downloads" ||
    route === "plugins"
  ) {
    try {
      storage.setItem(SHELL_LAST_ROUTE_KEY, route);
    } catch {
      // Storage unavailable or quota exceeded; fail-safe no-op
    }
  }
}

/**
 * Default browser localStorage instance guarded for SSR environments.
 */
export const shellStorage: SimpleStorage | null =
  typeof window !== "undefined" && window.localStorage
    ? window.localStorage
    : null;
