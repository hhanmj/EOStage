import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import path from "node:path";
import test, { after, before } from "node:test";

import { createServer, type ViteDevServer } from "vite";

type CredentialBridge = {
  clearCdseCredentials: () => Promise<Record<string, unknown>>;
  clearEarthdataCredentials: () => Promise<Record<string, unknown>>;
  clearUsgsCredentials: () => Promise<Record<string, unknown>>;
  getCredentialStatus: () => Promise<{
    cdse: string;
    usgs: string;
    earthdata: string;
    earthdata_input?: Record<string, string>;
  }>;
  saveCdseLogin: (username: string, password: string) => Promise<Record<string, unknown>>;
  saveEarthdataLogin: (
    username: string,
    password: string,
  ) => Promise<Record<string, unknown>>;
  saveEarthdataToken: (token: string) => Promise<Record<string, unknown>>;
  saveUsgsLogin: (username: string, applicationToken: string) => Promise<Record<string, unknown>>;
};

let server: ViteDevServer;
let bridge: CredentialBridge;

before(async () => {
  const uiRoot = fileURLToPath(new URL("../../", import.meta.url));
  server = await createServer({
    root: uiRoot,
    configFile: path.join(uiRoot, "vite.config.ts"),
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  bridge = (await server.ssrLoadModule("/src/lib/bridge.ts")) as CredentialBridge;
});

after(async () => {
  await server.close();
});

test("Browser Fixture token save and status never serialize the submitted token", async () => {
  const sentinel = "SENTINEL_BROWSER_TOKEN";
  await bridge.clearEarthdataCredentials();

  const saved = await bridge.saveEarthdataToken(sentinel);
  const status = await bridge.getCredentialStatus();
  const serialized = JSON.stringify({ saved, status });

  assert.equal(saved.ok, true);
  assert.equal(status.earthdata, "token");
  assert.deepEqual(status.earthdata_input, {
    mode: "token",
    token: "",
    username: "",
    password: "",
  });
  assert.equal(serialized.includes(sentinel), false);
});

test("Browser Fixture login save and status retain only a bounded mask", async () => {
  const username = "sentinel-user";
  const password = "SENTINEL_BROWSER_PASSWORD";
  await bridge.clearEarthdataCredentials();

  const saved = await bridge.saveEarthdataLogin(username, password);
  const status = await bridge.getCredentialStatus();
  const serialized = JSON.stringify({ saved, status });

  assert.equal(saved.ok, true);
  assert.equal(status.earthdata, "login:s***");
  assert.deepEqual(status.earthdata_input, {
    mode: "login",
    token: "",
    username: "",
    password: "",
  });
  assert.equal(serialized.includes(username), false);
  assert.equal(serialized.includes(password), false);
});

test("Browser Fixture CDSE and USGS status retain only bounded masks", async () => {
  const cdseUser = "cdse-sentinel-user";
  const cdsePassword = "SENTINEL_CDSE_PASSWORD";
  const usgsUser = "usgs-sentinel-user";
  const usgsToken = "SENTINEL_USGS_APPLICATION_TOKEN";
  await bridge.clearCdseCredentials();
  await bridge.clearUsgsCredentials();

  const cdseSaved = await bridge.saveCdseLogin(cdseUser, cdsePassword);
  const usgsSaved = await bridge.saveUsgsLogin(usgsUser, usgsToken);
  const status = await bridge.getCredentialStatus();
  const serialized = JSON.stringify({ cdseSaved, usgsSaved, status });

  assert.equal(cdseSaved.ok, true);
  assert.equal(usgsSaved.ok, true);
  assert.equal(status.cdse, "login:c***");
  assert.equal(status.usgs, "login:u***");
  for (const secret of [cdseUser, cdsePassword, usgsUser, usgsToken]) {
    assert.equal(serialized.includes(secret), false, secret);
  }
});
