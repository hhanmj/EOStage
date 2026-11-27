import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const CONTROL_CHAR_OR_BACKSLASH_RE = /[\u0000-\u001f\u007f\\]/;

export function parseAtlasCdnBaseUrl(rawBase) {
  const trimmed = typeof rawBase === "string" ? rawBase.trim() : "";
  if (trimmed.length === 0) {
    return { kind: "empty", origin: null, normalized: "" };
  }
  if (CONTROL_CHAR_OR_BACKSLASH_RE.test(trimmed)) {
    throw new Error(
      `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": backslashes and control characters are not permitted.`
    );
  }
  if (trimmed.startsWith("//")) {
    throw new Error(
      `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": protocol-relative URLs are not permitted.`
    );
  }
  if (trimmed.startsWith("/")) {
    const normalized = trimmed.replace(/\/+$/, "");
    if (!normalized.startsWith("/") || normalized.startsWith("//")) {
      throw new Error(
        `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": relative atlas CDN path must stay same-origin.`
      );
    }
    let resolved;
    let probe;
    try {
      resolved = new URL(normalized, "https://eostage.invalid");
      probe = new URL(`${normalized}/v2/probe.webp`, "https://eostage.invalid");
    } catch {
      throw new Error(
        `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": invalid relative atlas CDN path.`
      );
    }
    if (
      resolved.origin !== "https://eostage.invalid" ||
      probe.origin !== "https://eostage.invalid" ||
      !resolved.pathname.startsWith("/") ||
      resolved.pathname.startsWith("//")
    ) {
      throw new Error(
        `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": relative atlas CDN path must resolve same-origin.`
      );
    }
    return {
      kind: "relative",
      origin: null,
      normalized,
    };
  }
  let parsed;
  try {
    parsed = new URL(trimmed);
  } catch {
    throw new Error(
      `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": expected an https:// URL or site-relative /path.`
    );
  }
  if (parsed.protocol !== "https:") {
    throw new Error(
      `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": desktop CSP requires https:// (got ${parsed.protocol}).`
    );
  }
  if (parsed.username || parsed.password) {
    throw new Error(
      `Invalid VITE_EO_ATLAS_CDN_BASE_URL "${trimmed}": embedded credentials are not permitted.`
    );
  }
  return {
    kind: "https",
    origin: parsed.origin,
    normalized: trimmed.replace(/\/+$/, ""),
  };
}

export function validateAtlasCdnAgainstCsp({ rawBase, imgSrc, configLabel }) {
  const parsed = parseAtlasCdnBaseUrl(rawBase);
  if (parsed.kind === "empty") {
    return parsed;
  }
  const list = Array.isArray(imgSrc) ? imgSrc : [];
  if (parsed.kind === "relative") {
    if (!list.includes("'self'")) {
      throw new Error(
        `${configLabel} img-src must allow 'self' for relative atlas CDN path "${parsed.normalized}".`
      );
    }
    return parsed;
  }
  if (!list.includes(parsed.origin)) {
    throw new Error(
      [
        `Atlas CDN origin "${parsed.origin}" (from VITE_EO_ATLAS_CDN_BASE_URL) is NOT allowlisted in ${configLabel} img-src.`,
        `Desktop WebView CSP would block loading onboarding imagery from this host.`,
        `To authorize this CDN origin consistently, allowlist "${parsed.origin}" in:`,
        `  1. src-tauri/tauri.conf.json (app.security.csp.img-src)`,
        `  2. src-tauri/tauri.dev.conf.json (app.security.devCsp.img-src)`,
        `  3. src-tauri/tests/security_policy.rs (expected image origins)`,
        `  4. ui/src/shell/api/w53Thumbnails.test.ts (CSP_ALLOWED_IMAGE_HOSTS alignment)`,
      ].join("\n")
    );
  }
  return parsed;
}

function readCspImgSrc(configPath, cspField) {
  const raw = JSON.parse(fs.readFileSync(configPath, "utf8"));
  return raw?.app?.security?.[cspField]?.["img-src"] ?? [];
}

export function assertDesktopAtlasCdnCspConsistency({
  repoRoot = REPO_ROOT,
  rawBase = process.env.VITE_EO_ATLAS_CDN_BASE_URL ?? "",
  mode = "all",
} = {}) {
  const prodConfig = path.join(repoRoot, "src-tauri", "tauri.conf.json");
  const devConfig = path.join(repoRoot, "src-tauri", "tauri.dev.conf.json");
  const parsed = validateAtlasCdnAgainstCsp({
    rawBase,
    imgSrc: readCspImgSrc(prodConfig, "csp"),
    configLabel: "src-tauri/tauri.conf.json (csp)",
  });
  if (mode === "serve" || mode === "all") {
    validateAtlasCdnAgainstCsp({
      rawBase,
      imgSrc: readCspImgSrc(devConfig, "devCsp"),
      configLabel: "src-tauri/tauri.dev.conf.json (devCsp)",
    });
  }
  return parsed;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const modeIdx = process.argv.indexOf("--mode");
  const mode = modeIdx >= 0 && process.argv[modeIdx + 1] ? process.argv[modeIdx + 1] : "all";
  const result = assertDesktopAtlasCdnCspConsistency({ mode });
  console.log(
    result.kind === "empty"
      ? "Atlas CDN disabled (unconfigured); desktop CSP consistency check passed."
      : `Atlas CDN "${result.normalized}" is explicitly allowlisted in desktop CSP.`
  );
}
