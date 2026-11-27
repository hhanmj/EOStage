import assert from "node:assert/strict";
import test from "node:test";

import {
  hasSavedProviderLogin,
  loginCdseCredentialSession,
  loginUsgsCredentialSession,
  saveCdseCredentialDraft,
  saveUsgsCredentialDraft,
} from "./providerCredentialSessions";

test("saved-login status is explicit and does not treat arbitrary configured states as login material", () => {
  assert.equal(hasSavedProviderLogin("login:f***"), true);
  assert.equal(hasSavedProviderLogin("token"), false);
  assert.equal(hasSavedProviderLogin("netrc"), false);
  assert.equal(hasSavedProviderLogin("none"), false);
  assert.equal(hasSavedProviderLogin(null), false);
});

test("CDSE draft validation fails closed before any native login call", async () => {
  assert.deepEqual(
    await saveCdseCredentialDraft("", "fixture-password"),
    { ok: false, error: "保存凭据需要同时填写 CDSE 用户名和密码。" },
  );
  assert.deepEqual(
    await loginCdseCredentialSession({
      savedStatus: "none",
      usernameDraft: "fixture-user",
      password: "",
      totpDraft: "",
    }),
    {
      ok: false,
      error: "当前 CDSE 草稿不完整；请同时填写用户名和密码，或清空草稿后使用已保存凭据登录。",
    },
  );
  assert.deepEqual(
    await loginCdseCredentialSession({
      savedStatus: "none",
      usernameDraft: "",
      password: "",
      totpDraft: "",
    }),
    { ok: false, error: "请输入 CDSE 用户名和密码，或先保存凭据后再登录。" },
  );
});

test("USGS draft validation requires username plus Application Token or a saved native login", async () => {
  assert.deepEqual(
    await saveUsgsCredentialDraft("fixture-user", ""),
    { ok: false, error: "保存凭据需要同时填写 USGS ERS 用户名和 Application Token。" },
  );
  assert.deepEqual(
    await loginUsgsCredentialSession({
      savedStatus: "none",
      usernameDraft: "",
      applicationToken: "fixture-token",
    }),
    {
      ok: false,
      error: "当前 USGS 草稿不完整；请同时填写用户名和 Application Token，或清空草稿后使用已保存凭据登录。",
    },
  );
  assert.deepEqual(
    await loginUsgsCredentialSession({
      savedStatus: "none",
      usernameDraft: "",
      applicationToken: "",
    }),
    { ok: false, error: "请输入 USGS ERS 用户名和 Application Token，或先保存凭据后再登录。" },
  );
});
