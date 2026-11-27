import assert from "node:assert/strict";
import test from "node:test";
import {
  projectProxyMode,
  proxyFieldsForMode,
  PROBE_ERROR_REASONS,
  TEST_ENDPOINTS,
} from "./networkSettingsLogic.ts";

test("proxy mode projection round-trips the persisted two-field state", () => {
  // direct: disabled proxy
  assert.equal(
    projectProxyMode({ proxy_enabled: false, proxy_url: "" }),
    "direct",
  );
  assert.equal(
    projectProxyMode({ proxy_enabled: false, proxy_url: "http://127.0.0.1:7890" }),
    "direct",
  );
  // system: enabled with an empty URL (the saved form of 自动识别系统代理)
  assert.equal(
    projectProxyMode({ proxy_enabled: true, proxy_url: "" }),
    "system",
  );
  assert.equal(
    projectProxyMode({ proxy_enabled: true, proxy_url: "   " }),
    "system",
  );
  // manual: enabled with a URL
  assert.equal(
    projectProxyMode({ proxy_enabled: true, proxy_url: "http://127.0.0.1:7890" }),
    "manual",
  );
});

test("saving the system mode persists an enabled proxy with an empty url", () => {
  // This is exactly the payload whose empty proxy_url the backend used to
  // reject with a URL parse error, silently losing the Owner's selection.
  assert.deepEqual(
    proxyFieldsForMode("system", { host: "127.0.0.1", port: "7890" }),
    { proxy_enabled: true, proxy_url: "" },
  );
  assert.deepEqual(
    proxyFieldsForMode("direct", { host: "127.0.0.1", port: "7890" }),
    { proxy_enabled: false, proxy_url: "" },
  );
  assert.deepEqual(
    proxyFieldsForMode("manual", { host: "127.0.0.1", port: "7890" }),
    { proxy_enabled: true, proxy_url: "http://127.0.0.1:7890" },
  );
  // A save -> load round trip through the projection keeps the same mode.
  for (const mode of ["direct", "system", "manual"] as const) {
    const fields = proxyFieldsForMode(mode, {
      host: "127.0.0.1",
      port: "7890",
    });
    assert.equal(projectProxyMode(fields), mode);
  }
});

test("probe endpoint list covers every provider with valid site links", () => {
  assert.deepEqual(
    TEST_ENDPOINTS.map((ep) => ep.id),
    [
      "cdse",
      "earthdata",
      "bhoonidhi",
      "usgs_m2m",
      "mpc",
      "opentopography",
      "gee",
      "gacos",
      "comet_licsar",
    ],
  );
  for (const ep of TEST_ENDPOINTS) {
    assert.ok(ep.name, `${ep.id} has a display name`);
    if (ep.id === "gacos") {
      assert.equal(ep.site, "http://www.gacos.net/");
    } else {
      assert.ok(ep.site.startsWith("https://"), `${ep.id} site must be https`);
    }
  }
});

test("every non-green probe error code has a hover reason", () => {
  for (const code of ["timeout", "transport_unavailable", "proxy_configuration_rejected"]) {
    assert.ok(PROBE_ERROR_REASONS[code], `${code} needs a reason`);
  }
});
