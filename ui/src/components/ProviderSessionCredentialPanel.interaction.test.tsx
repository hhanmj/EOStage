import assert from "node:assert/strict";
import test from "node:test";
import React from "react";

import { cleanup, render, setupUser } from "@/test-utils/render-harness";
import {
  CdseCredentialSessionPanel,
  UsgsCredentialSessionPanel,
} from "./ProviderSessionCredentialPanel";

test.afterEach(() => cleanup());

const callbacks = {
  onSave: () => undefined,
  onLogin: () => undefined,
  onRefresh: () => undefined,
  onClear: () => undefined,
  onLogout: () => undefined,
};

test("CDSE shared panel keeps draft validation, saved login, and runtime logout distinct", async () => {
  const user = setupUser();
  const calls: string[] = [];
  const actions = {
    onSave: () => calls.push("save"),
    onLogin: () => calls.push("login"),
    onRefresh: () => calls.push("refresh"),
    onClear: () => calls.push("clear"),
    onLogout: () => calls.push("logout"),
  };
  const view = render(
    <CdseCredentialSessionPanel
      credentialIdentity="fixture"
      saved={false}
      session={null}
      username=""
      password=""
      totp=""
      busy={null}
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onPasswordChange={() => undefined}
      onTotpChange={() => undefined}
      {...actions}
    />,
  );

  assert.equal((view.getByRole("button", { name: "保存凭据" }) as HTMLButtonElement).disabled, true);
  assert.equal((view.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled, true);
  assert.equal((view.getByRole("button", { name: "清除凭据" }) as HTMLButtonElement).disabled, true);
  assert.equal(view.queryByRole("button", { name: "退出会话" }), null);

  view.rerender(
    <CdseCredentialSessionPanel
      credentialIdentity="fixture"
      saved={false}
      session={null}
      username="fixture-user"
      password="fixture-password"
      totp="123456"
      busy={null}
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onPasswordChange={() => undefined}
      onTotpChange={() => undefined}
      {...actions}
    />,
  );
  await user.click(view.getByRole("button", { name: "保存凭据" }));
  await user.click(view.getByRole("button", { name: "登录" }));
  await user.click(view.getByRole("button", { name: "刷新状态" }));
  assert.deepEqual(calls, ["save", "login", "refresh"]);

  view.rerender(
    <CdseCredentialSessionPanel
      credentialIdentity="fixture"
      saved
      session={{ network_mode: "system", authenticated: false, refresh_available: false, expires_in_seconds: null }}
      username=""
      password=""
      totp=""
      busy={null}
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onPasswordChange={() => undefined}
      onTotpChange={() => undefined}
      {...actions}
    />,
  );
  const savedCdseSurface = view.container.querySelector('[data-provider-credential-surface="cdse"]');
  assert.match(savedCdseSurface?.textContent ?? "", /凭据已保存/);
  assert.doesNotMatch(savedCdseSurface?.textContent ?? "", /未登录/);

  view.rerender(
    <CdseCredentialSessionPanel
      credentialIdentity="fixture"
      saved
      session={{ network_mode: "system", authenticated: true, refresh_available: true, expires_in_seconds: 300 }}
      username=""
      password=""
      totp=""
      busy={null}
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onPasswordChange={() => undefined}
      onTotpChange={() => undefined}
      {...actions}
    />,
  );
  assert.equal((view.getByRole("button", { name: "重新登录" }) as HTMLButtonElement).disabled, false);
  assert.equal((view.getByRole("button", { name: "清除凭据" }) as HTMLButtonElement).disabled, false);
  const cdseSurface = view.container.querySelector('[data-provider-credential-surface="cdse"]');
  assert.match(cdseSurface?.textContent ?? "", /已保存/);
  assert.match(cdseSurface?.textContent ?? "", /会话可用/);
  await user.click(view.getByRole("button", { name: "退出会话" }));
  assert.equal(calls.at(-1), "logout");
});

test("USGS shared panel exposes Application Token semantics and saved-session actions", async () => {
  const user = setupUser();
  let loginCalls = 0;
  const view = render(
    <UsgsCredentialSessionPanel
      credentialIdentity="fixture"
      saved
      session={null}
      username=""
      applicationToken=""
      busy={null}
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onApplicationTokenChange={() => undefined}
      {...callbacks}
      onLogin={() => { loginCalls++; }}
    />,
  );

  assert.ok(view.getByLabelText("USGS M2M Application Token"));
  assert.equal((view.getByRole("button", { name: "保存凭据" }) as HTMLButtonElement).disabled, true);
  assert.equal((view.getByRole("button", { name: "登录" }) as HTMLButtonElement).disabled, false);
  assert.equal((view.getByRole("button", { name: "清除凭据" }) as HTMLButtonElement).disabled, false);
  const savedUsgsSurface = view.container.querySelector('[data-provider-credential-surface="usgs"]');
  assert.match(savedUsgsSurface?.textContent ?? "", /凭据已保存/);
  assert.doesNotMatch(savedUsgsSurface?.textContent ?? "", /未登录/);
  await user.click(view.getByRole("button", { name: "登录" }));
  assert.equal(loginCalls, 1);

  view.rerender(
    <UsgsCredentialSessionPanel
      credentialIdentity="fixture"
      saved
      session={{ state: "authenticated", authenticated: true }}
      username=""
      applicationToken=""
      busy="status"
      error={null}
      note={null}
      onUsernameChange={() => undefined}
      onApplicationTokenChange={() => undefined}
      {...callbacks}
    />,
  );
  for (const button of view.container.querySelectorAll("button")) {
    assert.equal((button as HTMLButtonElement).disabled, true);
  }
  const usgsSurface = view.container.querySelector('[data-provider-credential-surface="usgs"]');
  assert.match(usgsSurface?.textContent ?? "", /已保存/);
  assert.match(usgsSurface?.textContent ?? "", /会话可用/);
});
