/**
 * Single configuration point for EOStage's Stage-2 high-resolution onboarding EO image pack.
 *
 * In `DEV` mode, `/atlas-cdn` is enabled only when the optional repo-root `atlas-cdn-pack/v2/`
 * pack directory actually exists on disk (reported via `VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE` by Vite).
 * Production builds do not bundle `atlas-cdn-pack/`, so CDN URLs are enabled only when
 * `VITE_EO_ATLAS_CDN_BASE_URL` is configured with an allowlisted `https://` origin (or `/path`).
 */
export interface AtlasCdnEnvLike {
  DEV?: boolean;
  LOCAL_PACK_AVAILABLE?: boolean;
  VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE?: string;
  VITE_EO_ATLAS_CDN_BASE_URL?: string;
}

export interface AtlasCdnSettings {
  configured: boolean;
  baseUrl: string;
}

const CONTROL_CHAR_OR_BACKSLASH_RE = /[\u0000-\u001f\u007f\\]/;

function normalizeConfiguredAtlasBase(rawBase: string): string | null {
  const trimmed = rawBase.trim();
  if (
    trimmed.length === 0 ||
    CONTROL_CHAR_OR_BACKSLASH_RE.test(trimmed) ||
    trimmed.startsWith("//")
  ) {
    return null;
  }
  if (trimmed.startsWith("/")) {
    const normalized = trimmed.replace(/\/+$/, "");
    if (!normalized.startsWith("/") || normalized.startsWith("//")) {
      return null;
    }
    try {
      const resolved = new URL(normalized, "https://eostage.invalid");
      const probe = new URL(`${normalized}/v2/probe.webp`, "https://eostage.invalid");
      if (
        resolved.origin !== "https://eostage.invalid" ||
        probe.origin !== "https://eostage.invalid" ||
        !resolved.pathname.startsWith("/") ||
        resolved.pathname.startsWith("//")
      ) {
        return null;
      }
      return normalized;
    } catch {
      return null;
    }
  }
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
      return null;
    }
    return trimmed.replace(/\/+$/, "");
  } catch {
    return null;
  }
}

export function resolveAtlasCdnSettings(env?: AtlasCdnEnvLike): AtlasCdnSettings {
  const rawBase =
    typeof env?.VITE_EO_ATLAS_CDN_BASE_URL === "string"
      ? env.VITE_EO_ATLAS_CDN_BASE_URL
      : "";
  const normalizedBase = normalizeConfiguredAtlasBase(rawBase);
  if (normalizedBase) {
    return {
      configured: true,
      baseUrl: normalizedBase,
    };
  }
  if (rawBase.trim().length > 0) {
    return {
      configured: false,
      baseUrl: "",
    };
  }
  const hasLocalPack =
    env?.LOCAL_PACK_AVAILABLE === true ||
    env?.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE === "true";
  if (env?.DEV === true && hasLocalPack) {
    return {
      configured: true,
      baseUrl: "/atlas-cdn",
    };
  }
  return {
    configured: false,
    baseUrl: "",
  };
}

const RUNTIME_ENV: AtlasCdnEnvLike | undefined =
  typeof import.meta !== "undefined" && typeof import.meta.env !== "undefined"
    ? {
        DEV: Boolean(import.meta.env.DEV),
        LOCAL_PACK_AVAILABLE: import.meta.env.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE === "true",
        VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE:
          typeof import.meta.env.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE === "string"
            ? import.meta.env.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE
            : undefined,
        VITE_EO_ATLAS_CDN_BASE_URL:
          typeof import.meta.env.VITE_EO_ATLAS_CDN_BASE_URL === "string"
            ? import.meta.env.VITE_EO_ATLAS_CDN_BASE_URL
            : undefined,
      }
    : undefined;

const RESOLVED_ATLAS_CDN = resolveAtlasCdnSettings(RUNTIME_ENV);

export const EO_ATLAS_CDN_CONFIGURED = RESOLVED_ATLAS_CDN.configured;
export const EO_ATLAS_CDN_BASE_URL = RESOLVED_ATLAS_CDN.baseUrl;

export function resolveAtlasCdnUrl(relativePath: string): string {
  if (!EO_ATLAS_CDN_CONFIGURED || !EO_ATLAS_CDN_BASE_URL) {
    return "";
  }
  const trimmed = relativePath.replace(/^\/+/, "");
  return `${EO_ATLAS_CDN_BASE_URL}/${trimmed}`;
}
