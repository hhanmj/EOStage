import assert from "node:assert/strict";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  EO_ATLAS_CDN_CONFIGURED,
  resolveAtlasCdnSettings,
  resolveAtlasCdnUrl,
} from "../components/desktop/onboarding/atlasCdnConfig.ts";
import {
  assertDesktopAtlasCdnCspConsistency,
  validateAtlasCdnAgainstCsp,
} from "../../../scripts/check_atlas_cdn_csp.mjs";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

test("resolveAtlasCdnSettings disables /atlas-cdn unless local pack exists or valid CDN is configured", () => {
  assert.deepEqual(resolveAtlasCdnSettings({ DEV: false }), {
    configured: false,
    baseUrl: "",
  });
  assert.deepEqual(resolveAtlasCdnSettings({ DEV: false, VITE_EO_ATLAS_CDN_BASE_URL: "   " }), {
    configured: false,
    baseUrl: "",
  });
  assert.deepEqual(resolveAtlasCdnSettings({ DEV: true }), {
    configured: false,
    baseUrl: "",
  });
  assert.deepEqual(resolveAtlasCdnSettings({ DEV: true, LOCAL_PACK_AVAILABLE: false }), {
    configured: false,
    baseUrl: "",
  });
  assert.deepEqual(resolveAtlasCdnSettings({ DEV: true, LOCAL_PACK_AVAILABLE: true }), {
    configured: true,
    baseUrl: "/atlas-cdn",
  });
  assert.deepEqual(
    resolveAtlasCdnSettings({ DEV: true, VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE: "true" }),
    {
      configured: true,
      baseUrl: "/atlas-cdn",
    }
  );
  assert.deepEqual(
    resolveAtlasCdnSettings({ DEV: true, VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE: "false" }),
    {
      configured: false,
      baseUrl: "",
    }
  );
  assert.deepEqual(
    resolveAtlasCdnSettings({
      DEV: false,
      VITE_EO_ATLAS_CDN_BASE_URL: "https://cdn.example.com/eo-atlas///",
    }),
    {
      configured: true,
      baseUrl: "https://cdn.example.com/eo-atlas",
    }
  );
  for (const unsafe of [
    "http://cdn.example.com/eo-atlas",
    "//cdn.example.com/eo-atlas",
    "/\\cdn.example.com/eo-atlas",
    "/\t/cdn.example.com/eo-atlas",
    "/\n/cdn.example.com/eo-atlas",
    "/",
    "javascript:alert(1)",
    "https://user:pass@cdn.example.com/eo-atlas",
  ]) {
    assert.deepEqual(
      resolveAtlasCdnSettings({ DEV: true, LOCAL_PACK_AVAILABLE: true, VITE_EO_ATLAS_CDN_BASE_URL: unsafe }),
      { configured: false, baseUrl: "" },
      `unsafe CDN value must fail closed: ${unsafe}`
    );
    assert.throws(
      () =>
        assertDesktopAtlasCdnCspConsistency({
          repoRoot: REPO_ROOT,
          rawBase: unsafe,
          mode: "all",
        }),
      /Invalid VITE_EO_ATLAS_CDN_BASE_URL/,
      `CSP consistency gate must reject unsafe CDN value: ${unsafe}`
    );
  }
  assert.deepEqual(
    assertDesktopAtlasCdnCspConsistency({
      repoRoot: REPO_ROOT,
      rawBase: "/atlas-cdn",
      mode: "all",
    }),
    { kind: "relative", origin: null, normalized: "/atlas-cdn" }
  );
});

test("resolveAtlasCdnUrl returns empty string when unconfigured", () => {
  assert.equal(EO_ATLAS_CDN_CONFIGURED, false);
  assert.equal(resolveAtlasCdnUrl("v2/stage1/deck-1-layer-0.webp?v=6"), "");
});

test("desktop CSP check rejects unlisted atlas CDN origins and allows allowlisted ones", () => {
  assert.deepEqual(
    assertDesktopAtlasCdnCspConsistency({ repoRoot: REPO_ROOT, rawBase: "", mode: "all" }),
    { kind: "empty", origin: null, normalized: "" }
  );
  assert.throws(
    () =>
      assertDesktopAtlasCdnCspConsistency({
        repoRoot: REPO_ROOT,
        rawBase: "https://cdn.example.com/eo-atlas",
        mode: "build",
      }),
    /NOT allowlisted/
  );
  assert.throws(
    () =>
      validateAtlasCdnAgainstCsp({
        rawBase: "http://cdn.example.com/eo-atlas",
        imgSrc: ["'self'"],
        configLabel: "test-csp",
      }),
    /https:\/\//
  );
  const allowlisted = validateAtlasCdnAgainstCsp({
    rawBase: "https://sentinel-cogs.s3.us-west-2.amazonaws.com/atlas///",
    imgSrc: ["'self'", "https://sentinel-cogs.s3.us-west-2.amazonaws.com"],
    configLabel: "test-csp",
  });
  assert.equal(allowlisted.kind, "https");
  assert.equal(allowlisted.origin, "https://sentinel-cogs.s3.us-west-2.amazonaws.com");
});
