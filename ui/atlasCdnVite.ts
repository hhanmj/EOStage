import fs from "node:fs";
import path from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";
import { loadEnv, type Plugin } from "vite";
import { assertDesktopAtlasCdnCspConsistency } from "../scripts/check_atlas_cdn_csp.mjs";

export const ATLAS_CDN_PACK_ROOT = path.resolve(import.meta.dirname, "../atlas-cdn-pack");
const REPO_ROOT = path.resolve(import.meta.dirname, "..");

export function hasLocalAtlasPack(): boolean {
  const stage2Dir = path.join(ATLAS_CDN_PACK_ROOT, "v2");
  return fs.existsSync(stage2Dir) && fs.statSync(stage2Dir).isDirectory();
}

function handleAtlasCdnRequest(
  req: IncomingMessage,
  res: ServerResponse,
  next: () => void
): void {
  const rawUrl = req.url ?? "";
  if (!rawUrl.startsWith("/atlas-cdn/")) {
    next();
    return;
  }
  const cleanRel = decodeURIComponent(rawUrl.slice("/atlas-cdn/".length).split("?")[0]);
  const targetPath = path.resolve(ATLAS_CDN_PACK_ROOT, cleanRel);
  if (
    targetPath !== ATLAS_CDN_PACK_ROOT &&
    !targetPath.startsWith(`${ATLAS_CDN_PACK_ROOT}${path.sep}`)
  ) {
    res.statusCode = 403;
    res.end("Forbidden");
    return;
  }
  if (!fs.existsSync(targetPath) || fs.statSync(targetPath).isDirectory()) {
    next();
    return;
  }
  const ext = path.extname(targetPath).toLowerCase();
  const contentType =
    ext === ".webp"
      ? "image/webp"
      : ext === ".json"
        ? "application/json; charset=utf-8"
        : ext === ".png"
          ? "image/png"
          : ext === ".jpg" || ext === ".jpeg"
            ? "image/jpeg"
            : "application/octet-stream";
  res.setHeader("Content-Type", contentType);
  res.setHeader("Cache-Control", "public, max-age=3600");
  fs.createReadStream(targetPath).pipe(res);
}

/**
 * Serves the standalone repo-root `atlas-cdn-pack/` folder at `/atlas-cdn/` during dev/preview ONLY
 * when the local pack is actually present on disk.
 */
export function createEoAtlasLocalCdnPlugin(enabled: boolean): Plugin {
  return {
    name: "eostage-atlas-local-cdn",
    configureServer(server) {
      if (enabled) {
        server.middlewares.use(handleAtlasCdnRequest);
      }
    },
    configurePreviewServer(server) {
      if (enabled) {
        server.middlewares.use(handleAtlasCdnRequest);
      }
    },
  };
}

export function configureAtlasCdnForVite(command: "build" | "serve", mode: string) {
  const env = loadEnv(mode, import.meta.dirname, "VITE_");
  const rawBase = env.VITE_EO_ATLAS_CDN_BASE_URL ?? process.env.VITE_EO_ATLAS_CDN_BASE_URL ?? "";
  assertDesktopAtlasCdnCspConsistency({
    repoRoot: REPO_ROOT,
    rawBase,
    mode: command === "serve" ? "serve" : "build",
  });
  const localPackAvailable = command === "serve" && hasLocalAtlasPack();
  process.env.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE = localPackAvailable ? "true" : "false";
  return {
    localPackAvailable,
    define: {
      "import.meta.env.VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE": JSON.stringify(
        localPackAvailable ? "true" : "false"
      ),
    },
  };
}
