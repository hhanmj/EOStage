import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render, setupUser } from "@/test-utils/render-harness";
import { EarthdataCredentialFields, EarthdataCredentialModeTabs, type EarthdataCredentialFieldsProps } from "./EarthdataCredentialFields";
import type { DesktopCredentialRevealResult } from "@/lib/desktopShellBridge";

test.afterEach(() => cleanup());
const base: EarthdataCredentialFieldsProps = {
  status: "token", credentialIdentity: "one", token: "", username: "", password: "",
  onTokenChange: () => undefined, onUsernameChange: () => undefined, onPasswordChange: () => undefined,
};

test("Earthdata credential mode tabs are shared, accessible, and lock while a credential action is busy", async () => {
  const user = setupUser();
  let mode: "token" | "login" = "token";
  const view = render(<EarthdataCredentialModeTabs value={mode} onChange={(next) => { mode = next; }} />);
  const token = view.getByRole("tab", { name: "Token" }) as HTMLButtonElement;
  const login = view.getByRole("tab", { name: "账号密码" }) as HTMLButtonElement;
  assert.equal(token.getAttribute("aria-selected"), "true");
  assert.equal(login.getAttribute("aria-selected"), "false");
  await user.click(login);
  assert.equal(mode, "login");

  view.rerender(<EarthdataCredentialModeTabs value={mode} onChange={(next) => { mode = next; }} disabled />);
  assert.equal((view.getByRole("tab", { name: "Token" }) as HTMLButtonElement).disabled, true);
  assert.equal((view.getByRole("tab", { name: "账号密码" }) as HTMLButtonElement).disabled, true);
});

test("token and login use shared inputs; saved reveal is display-only and deletion guidance is explicit", async () => {
  const user = setupUser();
  for (const mode of ["token", "login"] as const) {
    const fields = mode === "token" ? ["Earthdata Token"] : ["Earthdata 用户名", "Earthdata 密码"];
    const reads: string[] = [];
    const view = render(<EarthdataCredentialFields {...base} mode={mode}
      status={mode === "token" ? "token" : "login:fixture"} onReveal={async (field) => {
        reads.push(field); return { ok: true, value: "saved-fixture" };
      }} />);
    assert.equal(view.container.querySelectorAll("input").length, fields.length);
    assert.equal(reads.length, 0);
    for (const label of fields) {
      const input = view.getByLabelText(label) as HTMLInputElement;
      assert.equal(input.value, "");
      assert.equal(input.placeholder, "••••••••");
      await user.click(view.getByRole("button", { name: `显示已保存的 ${label}` }));
      assert.equal(input.value, "saved-fixture");
      assert.equal(input.readOnly, true);
    }
    assert.deepEqual(reads, mode === "token" ? ["earthdata_token"] : ["earthdata_username", "earthdata_password"]);
    assert.equal(view.queryByRole("button", { name: "保存 Earthdata Token" }), null);
    assert.equal(view.queryByRole("button", { name: "保存登录凭据" }), null);
    cleanup();
  }
});

test("field component never owns save or clear mutations", () => {
  for (const status of ["token", "login:fixture", "none"] as const) {
    const view = render(<EarthdataCredentialFields {...base} status={status} />);
    assert.equal(view.queryByRole("button", { name: "保存 Earthdata Token" }), null);
    assert.equal(view.queryByRole("button", { name: "保存登录凭据" }), null);
    assert.equal(view.queryByRole("button", { name: /清除凭据/ }), null);
    cleanup();
  }
});

test("external, unavailable and non-native sources do not request saved material", async () => {
  const user = setupUser();
  for (const status of ["env-token", "netrc", "none", "unavailable", null]) {
    let reads = 0;
    const view = render(<EarthdataCredentialFields {...base} status={status} onReveal={async () => {
      reads++; return { ok: true, value: "unreachable-fixture" };
    }} />);
    assert.equal(view.queryByRole("button", { name: /显示已保存/ }), null);
    await user.click(view.getByRole("button", { name: "显示 Earthdata Token" }));
    assert.equal(reads, 0);
    cleanup();
  }
  const view = render(<EarthdataCredentialFields {...base} />);
  assert.equal(view.queryByRole("button", { name: /显示已保存/ }), null);
});

test("draft editing stays in the parent and busy locks both modes", () => {
  let draft = "";
  const view = render(<EarthdataCredentialFields {...base} onTokenChange={(value) => { draft = value; }} />);
  fireEvent.change(view.getByLabelText("Earthdata Token"), { target: { value: "draft-fixture" } });
  assert.equal(draft, "draft-fixture");
  view.rerender(<EarthdataCredentialFields {...base} token={draft} username="fixture" password="draft-password" busy="token" />);
  for (const element of view.container.querySelectorAll("input,button")) {
    assert.equal((element as HTMLInputElement | HTMLButtonElement).disabled, true);
  }
});

test("changing credential mode invalidates a pending reveal without leaking into the replacement view", async () => {
  const user = setupUser();
  let resolve!: (value: DesktopCredentialRevealResult) => void;
  const onReveal = () => new Promise<DesktopCredentialRevealResult>((done) => { resolve = done; });
  const view = render(<EarthdataCredentialFields {...base} mode="token" onReveal={onReveal} />);
  await user.click(view.getByRole("button", { name: "显示已保存的 Earthdata Token" }));
  view.rerender(<EarthdataCredentialFields {...base} mode="login" status="login:fixture" onReveal={onReveal} />);
  await act(async () => { resolve({ ok: true, value: "late-fixture" }); });
  assert.equal(view.queryByLabelText("Earthdata Token"), null);
  for (const input of view.container.querySelectorAll("input")) assert.equal(input.value, "");
});
